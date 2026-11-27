package app

import (
	"net"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"testing"
)

func TestSystemdHTTPListener(t *testing.T) {
	const child = "LMM_TEST_INHERITED_HTTP_LISTENER"
	if address := os.Getenv(child); address != "" {
		t.Setenv("LISTEN_PID", strconv.Itoa(os.Getpid()))
		t.Setenv("LISTEN_FDS", "1")
		t.Setenv("LISTEN_FDNAMES", "lmm-extensions-http")
		listener, err := listenHTTP(address)
		if want := os.Getenv(child + "_ERROR"); want != "" {
			if err == nil || !strings.Contains(err.Error(), want) {
				t.Fatalf("want %q, got %v", want, err)
			}
			return
		}
		if err != nil {
			t.Fatal(err)
		}
		defer listener.Close()
		if listener.Addr().String() != address || os.Getenv("LISTEN_FDS") != "" {
			t.Fatalf("wrong inherited address or activation state: %v", listener.Addr())
		}
		return
	}
	server, err := net.ListenTCP("tcp4", &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1)})
	if err != nil {
		t.Fatal(err)
	}
	defer server.Close()
	file, err := server.File()
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	for _, tc := range []struct{ address, want string }{
		{server.Addr().String(), ""},
		{"127.0.0.1:0", "address does not match"},
	} {
		cmd := exec.Command(os.Args[0], "-test.run=^TestSystemdHTTPListener$")
		cmd.Env = append(os.Environ(), child+"="+tc.address, child+"_ERROR="+tc.want)
		cmd.ExtraFiles = []*os.File{file} // The child receives descriptor 3.
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("inherited listener fixture failed: %v\n%s", err, out)
		}
	}
}
