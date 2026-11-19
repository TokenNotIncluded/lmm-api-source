// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package testkit

import (
	"os"
	"strings"
	"testing"
)

// ClearPostgresEnv lets each test arrange its own PostgreSQL connection
// environment and restores all inherited values when the test finishes.
func ClearPostgresEnv(t testing.TB) {
	t.Helper()
	for _, entry := range os.Environ() {
		name, _, _ := strings.Cut(entry, "=")
		if !strings.HasPrefix(name, "PG") {
			continue
		}
		// Setenv registers restoration, including an originally empty value.
		t.Setenv(name, "")
		if err := os.Unsetenv(name); err != nil {
			t.Fatalf("clear PostgreSQL environment %s: %v", name, err)
		}
	}
}
