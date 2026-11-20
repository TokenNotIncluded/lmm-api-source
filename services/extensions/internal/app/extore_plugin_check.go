package app

import (
	"fmt"
	"io"
	"os"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/extore"
)

// checkExtorePlugin verifies the configured native images without assembling the
// runtime, reading business credentials, opening a database or issuing HTTP.
func checkExtorePlugin(args []string, output io.Writer) error {
	path, err := configPath(args)
	if err != nil {
		return err
	}
	if err = validateEnvironment(os.Getenv); err != nil {
		return err
	}
	cfg, err := LoadConfig(path)
	if err != nil {
		return err
	}
	reader, err := extore.NewNativeCatalogReader(cfg.Store.ExtorePlugin)
	if err != nil {
		return err
	}
	if err = reader.Close(); err != nil {
		return err
	}
	_, err = fmt.Fprintln(output, "Extore native plugin loaded: host ABI 1, extore.commerce v1")
	return err
}
