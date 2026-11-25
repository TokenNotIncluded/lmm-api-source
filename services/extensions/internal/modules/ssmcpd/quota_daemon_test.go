package ssmcpd

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// Set SSMCPD_TEST_BINARY to the accounting-enabled daemon to check its wire
// contract with the Go client. The fixture owns its port, state and identities.
func TestQuotaAccountingDaemon(t *testing.T) {
	binary := os.Getenv("SSMCPD_TEST_BINARY")
	if binary == "" {
		t.Skip("set SSMCPD_TEST_BINARY to run the daemon integration test")
	}
	dir := t.TempDir()
	listener, err := net.Listen("tcp4", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	address := listener.Addr().String()
	if err := listener.Close(); err != nil {
		t.Fatal(err)
	}
	serviceToken := strings.Repeat("e", 64)
	digest := func(raw string) string { return fmt.Sprintf("%x", sha256.Sum256([]byte(raw))) }
	config := fmt.Sprintf(`[server]
listen = %q
public_url = "https://relay.invalid"
[relay]
device_ingress_sha256 = %q
[relay.principals]
alice = %q
[relay.accounting]
issuer = "https://example.invalid"
state_dir = %q
service_token_sha256 = %q
utc_offset_minutes = 480
free_monthly_bytes = 12
paid_monthly_bytes = 24
[relay.accounting.users]
alice = "123"
`, address, digest(strings.Repeat("d", 64)), digest(strings.Repeat("a", 64)), filepath.Join(dir, "state"), digest(serviceToken))
	configFile := filepath.Join(dir, "fixture.toml")
	tokenFile := filepath.Join(dir, "fixture-token")
	if err := os.WriteFile(configFile, []byte(config), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(tokenFile, []byte(serviceToken), 0600); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, binary, "serve", "--config", configFile)
	cmd.Dir = dir
	cmd.Env = []string{"HOME=" + dir}
	var output bytes.Buffer
	cmd.Stderr = &output
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() { done <- cmd.Wait(); close(done) }()
	defer func() { cancel(); <-done }()
	s, err := New(Config{Mode: "relay", PublicOrigin: "https://api.lmm.best", Metering: &InternalConfig{Origin: "http://" + address, TokenFile: tokenFile}})
	if err != nil {
		t.Fatal(err)
	}
	var q *Quota
	for {
		q, err = s.Quota(ctx, 123)
		if err == nil {
			break
		}
		select {
		case err := <-done:
			t.Fatalf("daemon exited: %v %s", err, output.String())
		case <-ctx.Done():
			t.Fatal("quota request timed out")
		case <-time.After(100 * time.Millisecond):
		}
	}
	if q.Username != "alice" || q.UserID != "123" || q.UsedBytes != "0" || q.FreeBytes != "12" || q.LimitBytes == nil || *q.LimitBytes != "12" || q.RemainingBytes == nil || *q.RemainingBytes != "12" {
		t.Fatalf("unexpected metering result: %+v", q)
	}
	if _, err = s.Quota(ctx, 789); !errors.Is(err, ErrUnbound) {
		t.Fatalf("unknown fixture actor: %v", err)
	}
}
