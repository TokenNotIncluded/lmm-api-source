package composition

import (
	"bytes"
	"errors"
	"os"
	"path/filepath"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
)

// replacePancakeConfiguration commits a previously validated private document.
// The caller serializes configuration changes and prepares the runtime snapshot
// before calling this function. expected prevents overwriting an operator edit.
func replacePancakeConfiguration(path string, expected, next []byte) error {
	if !cleanPaymentPath(path) || len(next) == 0 || len(next) > 64<<10 {
		return errors.New("invalid payment configuration replacement")
	}
	current, err := coreclient.ReadPrivateFile(path, 64<<10)
	if err != nil || !bytes.Equal(current, expected) {
		return errors.New("payment configuration changed")
	}
	file, err := os.CreateTemp(filepath.Dir(path), ".pancake-config-*")
	if err != nil {
		return errors.New("payment configuration staging unavailable")
	}
	name := file.Name()
	defer os.Remove(name)
	defer file.Close()
	if err = file.Chmod(0600); err != nil {
		return err
	}
	if _, err = file.Write(next); err != nil {
		return err
	}
	if err = file.Sync(); err != nil {
		return err
	}
	if err = file.Close(); err != nil {
		return err
	}
	current, err = coreclient.ReadPrivateFile(path, 64<<10)
	if err != nil || !bytes.Equal(current, expected) {
		return errors.New("payment configuration changed")
	}
	directory, err := os.Open(filepath.Dir(path))
	if err != nil {
		return err
	}
	defer directory.Close()
	if err = os.Rename(name, path); err != nil {
		return err
	}
	return directory.Sync()
}
