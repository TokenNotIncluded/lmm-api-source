// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"net"
	"net/url"
	"regexp"
	"strconv"
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

// normalizeDirectoryAdInput owns publication policy for drafts and placements
// stored in the isolated community database. Core retains only consent binding.
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
	if port := u.Port(); port != "" {
		if _, err := strconv.ParseUint(port, 10, 16); err != nil {
			return in, errAdInvalid
		}
	}
	host := strings.ToLower(u.Hostname())
	if ip := net.ParseIP(host); ip != nil {
		if !publicDirectoryAdIP(ip) {
			return in, errAdInvalid
		}
	} else {
		if !strings.Contains(host, ".") || strings.HasSuffix(host, ".") || strings.ContainsAny(host, ":%") {
			return in, errAdInvalid
		}
		for _, suffix := range []string{".localhost", ".local", ".internal", ".test"} {
			if strings.HasSuffix(host, suffix) {
				return in, errAdInvalid
			}
		}
		// Browsers interpret numeric final labels as legacy IPv4 spellings.
		// Require canonical IP literals so the publication rule cannot be
		// bypassed with abbreviated, octal or hexadecimal loopback addresses.
		last := host[strings.LastIndexByte(host, '.')+1:]
		if _, err := strconv.ParseUint(last, 0, 32); err == nil ||
			strings.Trim(last, "0123456789") == "" {
			return in, errAdInvalid
		}
	}
	return in, nil
}

// Publication eligibility only. This never authorizes a Core charge or refund.
func publicDirectoryAdIP(ip net.IP) bool {
	if !ip.IsGlobalUnicast() || ip.IsPrivate() {
		return false
	}
	if v4 := ip.To4(); v4 != nil {
		a, b, c := v4[0], v4[1], v4[2]
		return a != 0 && a < 224 &&
			!(a == 100 && b >= 64 && b <= 127) &&
			!(a == 198 && (b == 18 || b == 19)) &&
			!(a == 192 && b == 0 && c == 2) &&
			!(a == 198 && b == 51 && c == 100) &&
			!(a == 203 && b == 0 && c == 113)
	}
	return ip[0]&0xe0 == 0x20 &&
		!(ip[0] == 0x20 && ip[1] == 0x01 && ip[2] == 0x0d && ip[3] == 0xb8)
}

func directoryAdContentHash(in directoryAdInput) string {
	h := sha256.New()
	_, _ = h.Write([]byte("lmm.directory-ad.content.v1\n"))
	for _, value := range []string{in.Name, in.URL, in.Summary, in.Description} {
		_, _ = fmt.Fprintf(h, "%d:%s\n", len(value), value)
	}
	return hex.EncodeToString(h.Sum(nil))
}
