//go:build !unix

package extore

import "context"

func checkPluginSocket(string) error { return ErrUnavailable }

func StartPlugin(context.Context, PluginConfig) (*PluginHost, error) {
	return nil, ErrUnavailable
}
