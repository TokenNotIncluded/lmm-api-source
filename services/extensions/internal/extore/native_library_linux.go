//go:build linux && (amd64 || arm64)

package extore

import (
	"errors"
	"fmt"

	"github.com/ebitengine/purego"
)

func loadNativeAPI(path string) (*nativeAPI, error) {
	// The host and zplugin images are pinned for process lifetime. Unloading
	// code while Rust thread-local state exists would invalidate its destructors.
	library, err := purego.Dlopen(path, purego.RTLD_NOW|purego.RTLD_LOCAL)
	if err != nil {
		return nil, errors.New("Extore native host library could not be loaded")
	}
	api := &nativeAPI{}
	for _, symbol := range []struct {
		name string
		fn   any
	}{
		{"extore_host_abi_version", &api.version},
		{"extore_host_open", &api.open},
		{"extore_host_catalog", &api.catalog},
		{"extore_host_release", &api.release},
		{"extore_host_close", &api.close},
	} {
		address, err := purego.Dlsym(library, symbol.name)
		if err != nil {
			return nil, fmt.Errorf("Extore native host is missing %s", symbol.name)
		}
		purego.RegisterFunc(symbol.fn, address)
	}
	return api, nil
}
