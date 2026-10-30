// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package payments

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strconv"
	"strings"
)

func validProviderCredential(value string) bool {
	if len(value) < 16 || len(value) > 4096 {
		return false
	}
	for _, c := range []byte(value) {
		if c < 33 || c > 126 {
			return false
		}
	}
	return true
}

func validTopupOrderID(id string) bool {
	n, err := strconv.ParseInt(strings.TrimPrefix(id, "top_"), 10, 64)
	return err == nil && n > 0 && id == "top_"+strconv.FormatInt(n, 10)
}

func providerOrderMatches(o Order, c Channel) bool {
	return o.ChannelID == c.ID && o.Intent.ID > 0 && o.ID == "top_"+strconv.FormatInt(o.Intent.ID, 10) && o.Intent.Provider == c.Provider &&
		o.Intent.Merchant == c.Merchant && o.Intent.Environment == c.Environment && o.Intent.Currency == c.Currency && validAmount(o.Intent.AmountMinor)
}

func callbackBytes(r *http.Request, header string) ([]byte, []byte, error) {
	if r.Method != http.MethodPost || r.URL.RawQuery != "" || len(r.Header.Values(header)) != 1 {
		return nil, nil, ErrInvalid
	}
	signature := r.Header.Get(header)
	if len(signature) == 0 || len(signature) > 2048 {
		return nil, nil, ErrInvalid
	}
	raw, err := boundedBytes(r.Body)
	if err != nil {
		return nil, nil, err
	}
	return raw, []byte(signature), nil
}

func requestJSONObject(ctx context.Context, client *http.Client, method, endpoint string, value any, headers http.Header) ([]byte, error) {
	var body []byte
	var err error
	if value != nil {
		body, err = json.Marshal(value)
		if err != nil {
			return nil, ErrInvalid
		}
	}
	return requestJSONBytes(ctx, client, method, endpoint, body, headers)
}

func requestJSONBytes(ctx context.Context, client *http.Client, method, endpoint string, body []byte, headers http.Header) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, method, endpoint, bytes.NewReader(body))
	if err != nil {
		return nil, ErrInvalid
	}
	req.Header = headers.Clone()
	req.Header.Set("Content-Type", "application/json")
	res, err := client.Do(req)
	if err != nil {
		return nil, ErrUnavailable
	}
	defer res.Body.Close()
	raw, err := boundedBytes(res.Body)
	if err != nil {
		return nil, err
	}
	if res.StatusCode < 200 || res.StatusCode >= 300 {
		return nil, ErrUnavailable
	}
	return raw, nil
}

// The SDK uses its supplied HTTP client. Bound and validate responses before
// its decoder reads them; retain the host's DNS, redirect and timeout policy.
type boundedProviderTransport struct{ transport http.RoundTripper }

func (t boundedProviderTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	response, err := t.transport.RoundTrip(r)
	if err != nil {
		return nil, err
	}
	raw, err := boundedBytes(response.Body)
	response.Body.Close()
	if err != nil {
		return nil, err
	}
	var object any
	if jsonObject(raw, &object) != nil {
		return nil, ErrInvalid
	}
	response.Body = io.NopCloser(bytes.NewReader(raw))
	return response, nil
}
