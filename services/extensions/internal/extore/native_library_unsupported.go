//go:build !linux || (!amd64 && !arm64)

package extore

import "errors"

func loadNativeAPI(string) (*nativeAPI, error) {
	return nil, errors.New("Extore native catalog requires Linux amd64 or arm64")
}
