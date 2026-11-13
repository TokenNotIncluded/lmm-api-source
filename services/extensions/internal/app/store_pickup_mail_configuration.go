package app

import (
	"errors"
	"slices"
)

func validateStorePickupMail(c GoConfig) error {
	m := c.Store.PickupMail
	if !m.Enabled() {
		return nil
	}
	if m.Validate() != nil || c.Store.DatabaseName == "" || c.Store.DatabaseURLFile == "" || !slices.Contains(c.Host.Modules, "store") {
		return errors.New("store pickup_mail requires complete TLS SMTP configuration and selected persistent store")
	}
	for _, p := range []string{m.KeyFile, m.PasswordFile, m.CAFile} {
		if p != "" && !absolutePath(p) {
			return errors.New("store pickup mail files must be absolute private paths")
		}
	}
	for _, p := range []string{c.Host.TokenFile, c.Core.IdentityTokenFile, c.Core.CommerceTokenFile, c.Core.PaymentTokenFile, c.Core.AssistantTokenFile, c.Core.DirectoryAdTokenFile, c.Core.UserReadTokenFile, c.Core.CheckinTokenFile, c.Store.StockKeyFile, c.Store.ExtoreKeyFile, c.Store.DatabaseURLFile, c.Assistant.CursorKeyFile, c.ToolMarket.EncryptionKeyFile, c.ToolMarket.CoreTokenFile, c.Scripts.RepositoryKeyFile, c.IoNet.KeyFile, m.PasswordFile, m.CAFile} {
		if p != "" && p == m.KeyFile {
			return errors.New("store pickup mail key must use a dedicated private file")
		}
	}
	return nil
}
