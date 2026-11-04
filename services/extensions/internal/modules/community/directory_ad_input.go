// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"net"
	"net/url"
	"regexp"
	"strings"
	"unicode"
	"unicode/utf8"
)

const (
	adMinBidCents int64 = 100
	adMaxBidCents int64 = 1_000_000
)

var adDigest = regexp.MustCompile(`^[0-9a-f]{64}$`)

type directoryAdInput struct {
	Name          string `json:"name"`
	URL           string `json:"url"`
	Summary       string `json:"summary"`
	Description   string `json:"description"`
	BidCents      int64  `json:"bid_cents"`
	ExpectedQuota int64  `json:"expected_quota"`
	RequestID     string `json:"request_id"`
	QuoteID       string `json:"quote_id"`
	ContentSHA256 string `json:"content_sha256"`
}

// normalizeDirectoryAdInput preserves the legacy advertisement input contract.
// It does not establish a quote, confirm a payment, or authorize a refund.
func normalizeDirectoryAdInput(in directoryAdInput) (directoryAdInput, error) {
	in.Name = strings.Trim(in.Name, " \t\r\n")
	in.URL = strings.Trim(in.URL, " \t\r\n")
	in.Summary = strings.Trim(in.Summary, " \t\r\n")
	in.Description = strings.Trim(in.Description, " \t\r\n")
	for _, value := range []string{in.Name, in.URL, in.Summary, in.Description} {
		if !utf8.ValidString(value) {
			return in, errAdInvalid
		}
		for _, r := range value {
			if r < 0x20 || r == 0x7f {
				return in, errAdInvalid
			}
		}
	}
	if in.Name == "" || len([]rune(in.Name)) > 80 || len([]rune(in.Summary)) > 180 ||
		len([]rune(in.Description)) > 1200 || !adRequestID.MatchString(in.RequestID) ||
		in.BidCents < adMinBidCents || in.BidCents > adMaxBidCents ||
		len(in.URL) == 0 || len(in.URL) > 2048 || strings.IndexFunc(in.URL, unicode.IsSpace) >= 0 {
		return in, errAdInvalid
	}
	u, err := url.Parse(in.URL)
	if err != nil || u.Scheme != "https" || u.Hostname() == "" || u.User != nil {
		return in, errAdInvalid
	}
	host := strings.ToLower(u.Hostname())
	if host == "localhost" || strings.HasSuffix(host, ".local") || !strings.Contains(host, ".") {
		return in, errAdInvalid
	}
	if ip := net.ParseIP(host); ip != nil && (ip.IsLoopback() || ip.IsPrivate() || ip.IsLinkLocalUnicast() || ip.IsUnspecified()) {
		return in, errAdInvalid
	}
	return in, nil
}

func directoryAdContentHash(in directoryAdInput) string {
	h := sha256.New()
	_, _ = h.Write([]byte("lmm.directory-ad.content.v1\n"))
	for _, value := range []string{in.Name, in.URL, in.Summary, in.Description} {
		_, _ = fmt.Fprintf(h, "%d:%s\n", len(value), value)
	}
	return hex.EncodeToString(h.Sum(nil))
}
