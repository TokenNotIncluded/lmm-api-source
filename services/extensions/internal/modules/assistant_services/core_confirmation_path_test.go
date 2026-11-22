package assistant_services

import (
	"strings"
	"testing"
)

func TestConfirmationBrowserPathProjectsOnlyExactHistoricalNavigation(t *testing.T) {
	id := strings.Repeat("a", 64)
	current := "/api/user/auth/assistant/confirmations/" + id
	for _, stored := range []string{current, "/api/user/assistant/confirmations/" + id} {
		got, err := currentConfirmationBrowserPath(stored, id)
		if err != nil || got != current {
			t.Fatalf("exact same-challenge path was not projected: %q %v", got, err)
		}
	}
	for _, stored := range []string{
		current + "?next=/",
		current + "#approve",
		current + "/key",
		"https://app.example" + current,
		"//app.example" + current,
		"/api/user/assistant/confirmations/" + strings.Repeat("b", 64),
		"/api/user/auth/assistant/confirmations/%2e%2e/" + id,
	} {
		if _, err := currentConfirmationBrowserPath(stored, id); err == nil {
			t.Fatalf("accepted non-exact browser path %q", stored)
		}
	}
	for _, invalid := range []string{"", strings.Repeat("a", 63), strings.Repeat("A", 64), id + "/"} {
		if _, err := currentConfirmationBrowserPath("/api/user/auth/assistant/confirmations/"+invalid, invalid); err == nil {
			t.Fatalf("accepted invalid challenge %q", invalid)
		}
	}
}
