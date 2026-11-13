package composition

import (
	"errors"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/coreclient"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

func configureStorePickupMail(s *store.Service, c Config, credentials ...[]byte) error {
	cfg := c.StorePickupMail
	if cfg.Validate() != nil {
		return errors.New("invalid store pickup mail configuration")
	}
	key, err := coreclient.ReadPrivateFile(cfg.KeyFile, 32)
	if err != nil || len(key) != 32 {
		return errors.New("private store pickup mail key unavailable")
	}
	for _, path := range []string{c.StoreStockKeyFile, c.StoreExtoreKeyFile, c.AssistantCursorKeyFile, c.ToolMarketKeyFile, c.ScriptRepositoryKeyFile, c.IoNetKeyFile, c.DirectoryAdTokenFile, c.UserReadTokenFile, c.ToolMarketTokenFile, c.ToolMarketWitnessTokenFile, c.CheckinTokenFile, c.SupportNoteKeyFile} {
		if path == "" {
			continue
		}
		raw, e := coreclient.ReadPrivateFile(path, 8192)
		if e != nil {
			return errors.New("private module credential unavailable")
		}
		credentials = append(credentials, raw)
	}
	for _, other := range credentials {
		if independentRuntimeCredentials(key, other) != nil {
			return errors.New("store pickup mail key must be independent")
		}
	}

	var password, ca []byte
	if cfg.PasswordFile != "" {
		password, err = coreclient.ReadPrivateFile(cfg.PasswordFile, 4096)
		if err != nil {
			return errors.New("private SMTP password unavailable")
		}
		if independentRuntimeCredentials(key, password) != nil {
			return errors.New("mail key and SMTP password must be independent")
		}
	}
	if cfg.CAFile != "" {
		ca, err = coreclient.ReadPrivateFile(cfg.CAFile, 64*1024)
		if err != nil {
			return errors.New("private SMTP CA file unavailable")
		}
	}
	sender, err := store.NewPickupSMTP(cfg, string(password), ca)
	if err != nil {
		return errors.New("invalid store SMTP TLS configuration")
	}
	return s.WithPickupMail(sender, key, cfg.Origin)
}
