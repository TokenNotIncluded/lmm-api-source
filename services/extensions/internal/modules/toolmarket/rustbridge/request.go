// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package rustbridge

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strconv"

	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

type requestSpec struct {
	major         int
	action        string
	requestBytes  int
	responseBytes int64
}

func (a *Adapter) request(ctx context.Context, action, user string, in, out any) error {
	return a.requestVersion(ctx, 1, action, user, in, out)
}

func (a *Adapter) requestVersion(ctx context.Context, major int, action, user string, in, out any) error {
	spec := requestSpec{major: major, action: action, requestBytes: 16 << 10, responseBytes: 64 << 10}
	if major == 2 {
		spec.requestBytes = 32 << 10
		spec.responseBytes = 256 << 10
	}
	return a.requestWithSpec(ctx, spec, user, in, out)
}

func (a *Adapter) requestWithSpec(ctx context.Context, spec requestSpec, user string, in, out any) error {
	if a == nil || a.client == nil {
		return market.ErrUnavailable
	}
	raw, err := json.Marshal(in)
	if err != nil || len(raw) > spec.requestBytes {
		return market.ErrInvalid
	}
	major := strconv.Itoa(spec.major)
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://core-toolmarket/internal/tool-market/v"+major+"/"+spec.action, bytes.NewReader(raw))
	if err != nil {
		return market.ErrUnavailable
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+a.token)
	req.Header.Set("X-Lmm-Protocol", major)
	if user != "" {
		req.Header.Set("X-Lmm-User-Credential", user)
	}
	response, err := a.client.Do(req)
	if err != nil {
		return market.ErrUnavailable
	}
	defer response.Body.Close()
	body, err := io.ReadAll(io.LimitReader(response.Body, spec.responseBytes+1))
	if err != nil || int64(len(body)) > spec.responseBytes {
		return market.ErrUnavailable
	}
	if response.StatusCode != http.StatusOK {
		switch response.StatusCode {
		case 400:
			return market.ErrInvalid
		case 401:
			return market.ErrAuth
		case 403:
			return market.ErrDenied
		case 404:
			return market.ErrMissing
		case 409:
			return market.ErrConflict
		default:
			return market.ErrUnavailable
		}
	}
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.DisallowUnknownFields()
	if err = decoder.Decode(out); err != nil {
		return market.ErrUnavailable
	}
	var extra any
	if !errors.Is(decoder.Decode(&extra), io.EOF) {
		return market.ErrUnavailable
	}
	return nil
}
