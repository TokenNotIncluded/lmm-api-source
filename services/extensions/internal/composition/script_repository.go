package composition

import (
	"errors"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
)

func scriptRepositoryKey(cfg Config, credentials ...[]byte) ([]byte, error) {
	key, err := coreclient.ReadPrivateFile(cfg.ScriptRepositoryKeyFile, 32)
	if err != nil || len(key) != 32 {
		return nil, errors.New("private script repository key unavailable")
	}
	for _, path := range []string{cfg.StoreStockKeyFile, cfg.StoreExtoreKeyFile, cfg.AssistantCursorKeyFile, cfg.ToolMarketKeyFile} {
		if path == "" {
			continue
		}
		other, err := coreclient.ReadPrivateFile(path, 32)
		if err != nil || len(other) != 32 {
			return nil, errors.New("private module encryption key unavailable")
		}
		credentials = append(credentials, other)
	}
	for _, path := range []string{cfg.DirectoryAdTokenFile, cfg.UserReadTokenFile, cfg.ToolMarketTokenFile, cfg.ToolMarketWitnessTokenFile} {
		if path == "" {
			continue
		}
		other, err := coreclient.ReadTokenFile(path)
		if err != nil {
			return nil, errors.New("private module credential unavailable")
		}
		credentials = append(credentials, other)
	}
	builtins, err := loadToolMarketBuiltinTokens(cfg.ToolMarketBuiltins)
	if err != nil {
		return nil, err
	}
	credentials = append(credentials, builtins...)
	for _, other := range credentials {
		if independentRuntimeCredentials(key, other) != nil {
			return nil, errors.New("script repository key must be independent")
		}
	}
	return key, nil
}
