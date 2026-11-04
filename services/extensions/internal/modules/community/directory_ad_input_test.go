// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import "testing"

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
