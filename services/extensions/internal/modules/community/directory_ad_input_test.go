// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
)

func TestNormalizeDirectoryAdInput(t *testing.T) {
	valid := directoryAdInput{Name: " Example ", URL: " https://example.com/path ", Summary: " Summary ", Description: " Description ", BidCents: 100, ExpectedQuota: 1, RequestID: "abcdefghijklmnop"}
	got, err := normalizeDirectoryAdInput(valid)
	if err != nil || got.Name != "Example" || got.URL != "https://example.com/path" || got.Summary != "Summary" || got.Description != "Description" {
		t.Fatalf("normalization: %#v, %v", got, err)
	}
	if digest := directoryAdContentHash(got); digest != "63ce5e6345ac2793ddc8150c82433b09cdf2ffc9216e8dddc0b8b2ba3111a49e" {
		t.Fatalf("digest = %s", digest)
	}
	cases := []directoryAdInput{valid, valid, valid, valid, valid, valid}
	cases[0].URL = "http://example.com"
	cases[1].URL = "https://192.168.1.1"
	cases[2].URL = "https://user@example.com"
	cases[3].BidCents = 99
	cases[4].Name = "bad\x01name"
	cases[5].RequestID = "short"
	for _, in := range cases {
		if _, err := normalizeDirectoryAdInput(in); err == nil {
			t.Errorf("accepted invalid input: %#v", in)
		}
	}
}

// These rules used to be rechecked in Core. They now reject publication in
// extensions before a draft is stored or a Core purchase is prepared.
func TestDirectoryAdPublicationURLPolicy(t *testing.T) {
	in := directoryAdInput{Name: "Example", BidCents: 100, RequestID: "abcdefghijklmnop"}
	for _, target := range []string{
		"https://example.com/path", "https://EXAMPLE.com/path", "https://8.8.8.8/",
		"https://[2606:4700:4700::1111]/", "https://[::ffff:8.8.8.8]/",
	} {
		in.URL = target
		if _, err := normalizeDirectoryAdInput(in); err != nil {
			t.Errorf("public URL %q rejected: %v", target, err)
		}
	}
	for _, target := range []string{
		"https://example.com./", "https://service.internal/", "https://service.test/",
		"https://service.localhost/", "https://service.local/", "https://localhost/",
		"https://0.1.2.3/", "https://100.64.0.1/", "https://100.127.255.255/",
		"https://198.18.0.1/", "https://198.19.255.255/", "https://192.0.2.1/",
		"https://198.51.100.1/", "https://203.0.113.1/", "https://224.0.0.1/",
		"https://255.255.255.255/", "https://169.254.1.1/", "https://10.0.0.1/",
		"https://[2001:db8::1]/", "https://[fc00::1]/", "https://[fe80::1]/",
		"https://[::1]/", "https://[::ffff:127.0.0.1]/", "https://[::ffff:192.0.2.1]/",
		"https://127.1/", "https://0177.0.0.1/", "https://0x7f.0x1/",
		"https://example.com:65536/", "https://user:password@example.com/",
	} {
		in.URL = target
		if _, err := normalizeDirectoryAdInput(in); err == nil {
			t.Errorf("ineligible URL %q accepted", target)
		}
	}
}

func TestDirectoryAdPublicationTextBounds(t *testing.T) {
	in := directoryAdInput{Name: strings.Repeat("广", 80), URL: "https://example.com/", Summary: strings.Repeat("摘", 180), Description: strings.Repeat("述", 1200), BidCents: 100, RequestID: "abcdefghijklmnop"}
	if _, err := normalizeDirectoryAdInput(in); err != nil {
		t.Fatalf("character limits rejected: %v", err)
	}
	for _, field := range []string{"name", "summary", "description"} {
		over := in
		switch field {
		case "name":
			over.Name += "广"
		case "summary":
			over.Summary += "摘"
		case "description":
			over.Description += "述"
		}
		if _, err := normalizeDirectoryAdInput(over); err == nil {
			t.Errorf("oversized %s accepted", field)
		}
	}
}

func TestDirectoryAdPrepareRejectsPublicationBeforeStorageOrCore(t *testing.T) {
	h := &handlers{
		// No database: invalid publication must be rejected before any write.
		s:    &Service{adCore: &recoveryAdCore{}},
		auth: fakeAuth{"user": {UserId: 1, PlatformLevel: 1, CredentialKind: pb.CredentialKind_CREDENTIAL_KIND_SESSION}},
	}
	for _, target := range []string{"https://service.internal", "https://192.0.2.1", "https://[2001:db8::1]"} {
		input := directoryAdInput{Name: "Example", URL: target, BidCents: 100, RequestID: "abcdefghijklmnop"}
		body, err := json.Marshal(input)
		if err != nil {
			t.Fatal(err)
		}
		req := httptest.NewRequest(http.MethodPost, "/api/ai-directory/ads/prepare", strings.NewReader(string(body)))
		req.Header.Set("Authorization", "Bearer user")
		rec := httptest.NewRecorder()
		h.prepareAd(rec, req)
		if rec.Code != http.StatusUnprocessableEntity {
			t.Fatalf("URL %q: %d %s", target, rec.Code, rec.Body.String())
		}
	}
}
