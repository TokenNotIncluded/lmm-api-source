// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"os"
	"strings"
	"testing"
)

// The Rust integration test starts the actual Core router and isolated identity
// database, then invokes this consumer against its private Unix listener.
func TestCoreUserRolesLive(t *testing.T) {
	socket := os.Getenv("LMM_TEST_CORE_USER_SOCKET")
	if socket == "" {
		t.Skip("requires Rust-owned live Core fixture")
	}
	token := []byte("synthetic-community-purpose-token-20261104")
	client, err := NewCoreRead(socket, token)
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	ctx := context.Background()
	if err = client.ReadyUsers(ctx); err != nil {
		t.Fatal(err)
	}
	users, err := client.Users(ctx, []int64{1, 2, 3, 999})
	if err != nil {
		t.Fatal(err)
	}
	if len(users) != 3 || users[1].RegisteredAt == nil || *users[1].RegisteredAt <= 0 || users[1].PlatformRole != "user" || users[1].Active || users[2].PlatformRole != "admin" || users[3].PlatformRole != "superadmin" {
		t.Fatalf("live identity mismatch: %#v", users)
	}
	page, err := client.UserPage(ctx, 0, 2)
	if err != nil || len(page.Users) != 2 || !page.HasMore || page.NextID != 2 || page.Users[0].UserID != 1 || page.Users[1].PlatformRole != "admin" {
		t.Fatalf("first live page: %#v %v", page, err)
	}
	page, err = client.UserPage(ctx, page.NextID, 2)
	if err != nil || len(page.Users) != 1 || page.HasMore || page.NextID != 3 || page.Users[0].PlatformRole != "superadmin" {
		t.Fatalf("last live page: %#v %v", page, err)
	}
	if _, err = client.UserPage(ctx, -1, 100); err == nil {
		t.Fatal("negative cursor accepted")
	}

	wrong, err := NewCoreRead(socket, []byte(strings.Repeat("x", 40)))
	if err != nil {
		t.Fatal(err)
	}
	defer wrong.Close()
	if err = wrong.ReadyUsers(ctx); err == nil {
		t.Fatal("Core accepted another purpose token")
	}
}
