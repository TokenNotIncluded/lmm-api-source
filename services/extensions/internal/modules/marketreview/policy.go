// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package marketreview

import (
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"syscall"
	"time"

	"github.com/BurntSushi/toml"
)

// FilePolicy uses a stable sidecar lock, never the renamed settings inode.
// The private directory must be provisioned by installation. A missing settings
// document is off/off; reads never fabricate enabled defaults.
type FilePolicy struct{ path string }

func NewFilePolicy(path string) (*FilePolicy, error) {
	if !filepath.IsAbs(path) {
		return nil, ErrInvalid
	}
	dir, err := os.Lstat(filepath.Dir(path))
	if err != nil || !dir.IsDir() || dir.Mode().Perm()&0077 != 0 {
		return nil, ErrInvalid
	}
	return &FilePolicy{path: path}, nil
}

func (p *FilePolicy) locked(ctx context.Context, exclusive bool, fn func() error) error {
	if p == nil || ctx == nil {
		return ErrUnavailable
	}
	fd, err := syscall.Open(p.path+".lock", syscall.O_CREAT|syscall.O_RDWR|syscall.O_CLOEXEC|syscall.O_NOFOLLOW, 0600)
	if err != nil {
		return ErrUnavailable
	}
	f := os.NewFile(uintptr(fd), p.path+".lock")
	defer f.Close()
	info, err := f.Stat()
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm()&0077 != 0 {
		return ErrUnavailable
	}
	mode := syscall.LOCK_SH
	if exclusive {
		mode = syscall.LOCK_EX
	}
	tick := time.NewTicker(10 * time.Millisecond)
	defer tick.Stop()
	for {
		if err = ctx.Err(); err != nil {
			return err
		}
		err = syscall.Flock(fd, mode|syscall.LOCK_NB)
		if err == nil {
			break
		}
		if err != syscall.EWOULDBLOCK && err != syscall.EAGAIN {
			return ErrUnavailable
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-tick.C:
		}
	}
	defer syscall.Flock(fd, syscall.LOCK_UN)
	return fn()
}

func (p *FilePolicy) read() (Modes, error) {
	fd, err := syscall.Open(p.path, syscall.O_RDONLY|syscall.O_CLOEXEC|syscall.O_NOFOLLOW, 0)
	if err == syscall.ENOENT {
		return Modes{Off, Off}, nil
	}
	if err != nil {
		return Modes{}, ErrUnavailable
	}
	f := os.NewFile(uintptr(fd), p.path)
	defer f.Close()
	info, err := f.Stat()
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm()&0077 != 0 || info.Size() > 4096 {
		return Modes{}, ErrUnavailable
	}
	raw, err := io.ReadAll(io.LimitReader(f, 4097))
	if err != nil || len(raw) > 4096 {
		return Modes{}, ErrUnavailable
	}
	var value struct {
		Tool  Mode `toml:"tool_mode"`
		Store Mode `toml:"store_mode"`
	}
	meta, err := toml.Decode(string(raw), &value)
	m := Modes{value.Tool, value.Store}
	if err != nil || len(meta.Undecoded()) != 0 || !m.Valid() {
		return Modes{}, ErrUnavailable
	}
	return m, nil
}

func (p *FilePolicy) With(ctx context.Context, fn func(Modes) error) error {
	if fn == nil {
		return ErrInvalid
	}
	return p.locked(ctx, false, func() error {
		m, err := p.read()
		if err != nil {
			return err
		}
		return fn(m)
	})
}

func (p *FilePolicy) Save(ctx context.Context, m Modes) error {
	if !m.Valid() {
		return ErrInvalid
	}
	return p.locked(ctx, true, func() error {
		if _, err := p.read(); err != nil {
			return err
		}
		f, err := os.CreateTemp(filepath.Dir(p.path), ".market-review-*")
		if err != nil {
			return ErrUnavailable
		}
		defer os.Remove(f.Name())
		if _, err = fmt.Fprintf(f, "tool_mode = %q\nstore_mode = %q\n", m.Tool, m.Store); err == nil {
			err = f.Sync()
		}
		closeErr := f.Close()
		if err != nil || closeErr != nil {
			return ErrUnavailable
		}
		if err = os.Rename(f.Name(), p.path); err != nil {
			return ErrUnavailable
		}
		dir, err := os.Open(filepath.Dir(p.path))
		if err != nil {
			return ErrUnavailable
		}
		defer dir.Close()
		return dir.Sync()
	})
}
