// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"encoding/json"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strconv"
)

// WalletPresets stores presentation settings next to the host's writable state.
// Each currency has its own atomic replacement, so independent edits never
// rewrite another currency or the private payment provider configuration.
type WalletPresets struct{ Directory string }

var walletPresetCurrencies = [...]string{"USD", "CNY", "EUR", "LDC", "USDC"}

func (s WalletPresets) path(currency string) (string, error) {
	for _, supported := range walletPresetCurrencies {
		if currency == supported {
			if !filepath.IsAbs(s.Directory) {
				return "", ErrUnavailable
			}
			return filepath.Join(s.Directory, "wallet-presets-"+currency+".json"), nil
		}
	}
	return "", ErrInvalid
}

func validateWalletPresets(amounts []string) error {
	if amounts == nil || len(amounts) > 32 {
		return ErrInvalid
	}
	seen := make(map[int64]bool, len(amounts))
	for _, value := range amounts {
		amount, err := strconv.ParseInt(value, 10, 64)
		if err != nil || !validAmount(amount) || value != strconv.FormatInt(amount, 10) || seen[amount] {
			return ErrInvalid
		}
		seen[amount] = true
	}
	return nil
}

// Missing overrides retain the wallet's existing channel-specific defaults.
// An explicit empty list disables shortcuts but preserves custom amounts.
func (s WalletPresets) Read() (map[string][]string, error) {
	result := make(map[string][]string)
	if s.Directory == "" {
		return result, nil
	}
	for _, currency := range walletPresetCurrencies {
		path, err := s.path(currency)
		if err != nil {
			return nil, err
		}
		file, err := os.Open(path)
		if errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil {
			return nil, ErrUnavailable
		}
		raw, err := io.ReadAll(io.LimitReader(file, 2049))
		file.Close()
		var amounts []string
		if err != nil || len(raw) > 2048 || json.Unmarshal(raw, &amounts) != nil || validateWalletPresets(amounts) != nil {
			return nil, ErrUnavailable
		}
		result[currency] = amounts
	}
	return result, nil
}

func (s WalletPresets) Save(currency string, amounts []string) error {
	path, err := s.path(currency)
	if err != nil {
		return err
	}
	if err := validateWalletPresets(amounts); err != nil {
		return err
	}
	raw, err := json.Marshal(amounts)
	if err != nil {
		return err
	}
	dir, err := os.Open(filepath.Dir(path))
	if err != nil {
		return ErrUnavailable
	}
	defer dir.Close()
	tmp, err := os.CreateTemp(dir.Name(), ".wallet-presets-*")
	if err != nil {
		return ErrUnavailable
	}
	defer os.Remove(tmp.Name())
	if _, err = tmp.Write(append(raw, '\n')); err == nil {
		err = tmp.Sync()
	}
	closeErr := tmp.Close()
	if err != nil || closeErr != nil || os.Rename(tmp.Name(), path) != nil || dir.Sync() != nil {
		return ErrUnavailable
	}
	return nil
}
