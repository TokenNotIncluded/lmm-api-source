//go:build unix

package extore

import (
	"context"
	"errors"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"syscall"
	"time"
)

func pluginOwner(info os.FileInfo, rootAllowed bool) bool {
	stat, ok := info.Sys().(*syscall.Stat_t)
	return ok && (stat.Uid == uint32(os.Geteuid()) || (rootAllowed && stat.Uid == 0))
}

func pluginPrivateDirectory(path string) error {
	info, err := os.Lstat(path)
	if err != nil || !info.IsDir() || info.Mode().Perm()&0077 != 0 || !pluginOwner(info, false) {
		return errors.New("Extore plugin requires a service-owned private directory")
	}
	actual, err := filepath.EvalSymlinks(path)
	if err != nil || actual != path {
		return errors.New("Extore plugin private directory must not use symlinks")
	}
	return pluginDirectoryChain(filepath.Dir(path))
}

// Sticky ancestors such as /tmp protect the owned child entry from replacement.
// Every other directory must be owner-controlled; symlink components are rejected.
func pluginDirectoryChain(path string) error {
	for {
		info, err := os.Lstat(path)
		if err != nil || !info.IsDir() || !pluginOwner(info, true) ||
			(info.Mode().Perm()&0022 != 0 && info.Mode()&os.ModeSticky == 0) {
			return errors.New("Extore plugin directories must have owner-controlled ancestors")
		}
		parent := filepath.Dir(path)
		if parent == path {
			return nil
		}
		path = parent
	}
}

func checkPluginSocket(path string) error {
	if err := pluginPrivateDirectory(filepath.Dir(path)); err != nil {
		return err
	}
	info, err := os.Lstat(path)
	if err != nil || info.Mode()&os.ModeSocket == 0 || info.Mode().Perm() != 0600 || !pluginOwner(info, false) {
		return errors.New("Extore plugin socket must be service-owned and mode 0600")
	}
	return nil
}

func pluginExecutable(path string) error {
	info, err := os.Lstat(path)
	if err != nil || !info.Mode().IsRegular() || info.Mode().Perm()&0022 != 0 || !pluginOwner(info, true) {
		return errors.New("Extore plugin code must be an owner-controlled regular file")
	}
	return pluginDirectoryChain(filepath.Dir(path))
}

// StartPlugin starts one private worker, verifies readiness after actual plugin
// configuration, and joins it on Close. It grants no Core credentials.
func StartPlugin(ctx context.Context, cfg PluginConfig) (*PluginHost, error) {
	if err := cfg.Validate(); err != nil {
		return nil, err
	}
	if !cfg.Enabled() {
		return nil, ErrInput
	}
	for _, path := range []string{cfg.Host, cfg.Library} {
		if err := pluginExecutable(path); err != nil {
			return nil, err
		}
	}
	for _, path := range []string{cfg.DataDir, filepath.Dir(cfg.Socket)} {
		if err := pluginPrivateDirectory(path); err != nil {
			return nil, err
		}
	}
	if _, err := os.Lstat(cfg.Socket); !errors.Is(err, os.ErrNotExist) {
		return nil, errors.New("Extore plugin socket already exists")
	}
	lifetime, cancel := context.WithCancel(context.Background())
	command := exec.CommandContext(lifetime, cfg.Host,
		"--socket", cfg.Socket, "--library", cfg.Library, "--sha256", cfg.SHA256,
		"--data-dir", cfg.DataDir, "--origin", cfg.Origin)
	// The worker receives its Extore configuration, not the Go process's
	// credential environment or dynamic-loader overrides.
	command.Env = []string{"PATH=/usr/bin:/bin", "LANG=C.UTF-8"}
	command.Stdout, command.Stderr = io.Discard, os.Stderr
	command.Cancel = func() error { return command.Process.Signal(syscall.SIGTERM) }
	command.WaitDelay = 5 * time.Second
	if err := command.Start(); err != nil {
		cancel()
		return nil, errors.New("Extore plugin host could not start")
	}
	done := make(chan struct{})
	var exitErr error
	go func() {
		exitErr = command.Wait()
		close(done)
	}()
	transport := pluginTransport(cfg.Socket)
	var socketInfo os.FileInfo
	var closeOnce sync.Once
	host := &PluginHost{origin: cfg.Origin, transport: transport}
	host.close = func() error {
		closeOnce.Do(func() {
			transport.CloseIdleConnections()
			cancel()
			<-done
			if current, err := os.Lstat(cfg.Socket); err == nil && socketInfo != nil && os.SameFile(current, socketInfo) {
				_ = os.Remove(cfg.Socket)
			}
		})
		return nil
	}
	ready, stop := context.WithTimeout(ctx, 20*time.Second)
	defer stop()
	ticker := time.NewTicker(50 * time.Millisecond)
	defer ticker.Stop()
	client := &http.Client{Transport: transport, Timeout: 250 * time.Millisecond}
	for {
		request, _ := http.NewRequestWithContext(ready, http.MethodGet, "http://extore.plugin/health/ready", nil)
		request.Header.Set("X-Extore-Origin", cfg.Origin)
		response, err := client.Do(request)
		if err == nil {
			_, _ = io.Copy(io.Discard, io.LimitReader(response.Body, 1024))
			_ = response.Body.Close()
			if response.StatusCode == http.StatusOK {
				socketInfo, err = os.Lstat(cfg.Socket)
				if err == nil {
					return host, nil
				}
			}
		}
		select {
		case <-done:
			_ = host.Close()
			_ = exitErr // Never include plugin diagnostics or credentials in an API error.
			return nil, errors.New("Extore plugin host failed during startup")
		case <-ready.Done():
			_ = host.Close()
			return nil, errors.New("Extore plugin host readiness timed out")
		case <-ticker.C:
		}
	}
}
