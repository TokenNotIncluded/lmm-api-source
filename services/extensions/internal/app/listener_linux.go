package app

import (
	"errors"
	"net"
	"net/netip"
	"os"
	"strconv"
	"syscall"
)

func listenHTTP(address string) (net.Listener, error) {
	pid, hasPID := os.LookupEnv("LISTEN_PID")
	count, hasCount := os.LookupEnv("LISTEN_FDS")
	names, hasNames := os.LookupEnv("LISTEN_FDNAMES")
	_, hasOffset := os.LookupEnv("LISTEN_FDS_FIRST_FD")
	if !hasPID && !hasCount && !hasNames && !hasOffset {
		return net.Listen("tcp", address)
	}
	if pid != strconv.Itoa(os.Getpid()) || count != "1" || names != "lmm-extensions-http" || hasOffset {
		return nil, errors.New("expected one systemd public socket named lmm-extensions-http for this process")
	}
	for _, name := range []string{"LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES"} {
		if err := os.Unsetenv(name); err != nil {
			return nil, err
		}
	}
	const descriptor = 3 // Systemd socket activation's first inherited descriptor.
	file := os.NewFile(descriptor, "lmm-extensions-http")
	defer file.Close()
	syscall.CloseOnExec(descriptor)
	listening, err := syscall.GetsockoptInt(descriptor, syscall.SOL_SOCKET, syscall.SO_ACCEPTCONN)
	if err != nil || listening != 1 {
		return nil, errors.New("systemd public socket is not listening")
	}
	listener, err := net.FileListener(file)
	if err != nil {
		return nil, err
	}
	if !matchesHTTPAddress(listener.Addr(), address) {
		_ = listener.Close()
		return nil, errors.New("systemd public socket address does not match the configured HTTP listener")
	}
	return listener, nil
}

func matchesHTTPAddress(address net.Addr, configured string) bool {
	want, err := netip.ParseAddrPort(configured)
	got, ok := address.(*net.TCPAddr)
	if err != nil || !ok {
		return false
	}
	actual := got.AddrPort()
	return actual.Port() == want.Port() && actual.Addr().Unmap() == want.Addr().Unmap()
}
