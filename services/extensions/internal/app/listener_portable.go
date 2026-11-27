//go:build !linux

package app

import "net"

func listenHTTP(address string) (net.Listener, error) {
	return net.Listen("tcp", address)
}
