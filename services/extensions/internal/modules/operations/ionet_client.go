// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

const ioNetOrigin = "https://api.io.solutions/enterprise/v1/io-cloud/caas"
const ioNetResponseLimit = 1 << 20

// Fixed destination, existing public-address DNS pinning, no proxy/cookie jar
// and no redirects. Provider errors never include request/response secret data.
func (s *IoNet) request(ctx context.Context, key, method, endpoint string, body []byte) ([]byte, int, error) {
	if !validRepositoryToken(key) || !strings.HasPrefix(endpoint, "/") || strings.HasPrefix(endpoint, "//") {
		return nil, 0, ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, method, ioNetOrigin+endpoint, bytes.NewReader(body))
	if err != nil {
		return nil, 0, ErrInvalid
	}
	req.Header.Set("x-api-key", key)
	req.Header.Set("Accept", "application/json")
	req.Header.Set("User-Agent", "LMM API io.net management")
	if len(body) > 0 {
		req.Header.Set("Content-Type", "application/json")
	}
	client := *s.client
	client.Jar = nil
	client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	response, err := client.Do(req)
	if err != nil {
		return nil, 0, ErrIoNetUpstream
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, response.StatusCode, ErrIoNetUpstream
	}
	raw, err := io.ReadAll(io.LimitReader(response.Body, ioNetResponseLimit+1))
	if err != nil || len(raw) > ioNetResponseLimit {
		return nil, response.StatusCode, ErrIoNetUpstream
	}
	return raw, response.StatusCode, nil
}

func decodeIoNet(raw []byte) (any, error) {
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.UseNumber()
	var value any
	if decoder.Decode(&value) != nil || decoder.Decode(new(any)) != io.EOF {
		return nil, ErrIoNetUpstream
	}
	if object, ok := value.(map[string]any); ok {
		if data, found := object["data"]; found {
			return data, nil
		}
	}
	return value, nil
}

// Provider snapshots can contain container secrets. Never mirror credentials,
// registry passwords or secret environment values into the Web or journal result.
func redactIoNet(value any, key string) any {
	switch v := value.(type) {
	case map[string]any:
		out := make(map[string]any, len(v))
		for name, item := range v {
			switch strings.ToLower(name) {
			case "api_key", "x-api-key", "authorization", "registry_secret", "registry_password", "secret_env_variables", "access_token", "refresh_token":
				continue
			}
			out[name] = redactIoNet(item, key)
		}
		return out
	case []any:
		out := make([]any, len(v))
		for i, item := range v {
			out[i] = redactIoNet(item, key)
		}
		return out
	case string:
		if key != "" {
			return strings.ReplaceAll(v, key, "[redacted]")
		}
		return v
	default:
		return v
	}
}

func (s *IoNet) Read(ctx context.Context, endpoint string) (any, error) {
	_, key, err := s.providerKey(ctx)
	if err != nil {
		return nil, err
	}
	raw, _, err := s.request(ctx, key, http.MethodGet, endpoint, nil)
	if err != nil {
		return nil, err
	}
	value, err := decodeIoNet(raw)
	if err != nil {
		return nil, err
	}
	return redactIoNet(value, key), nil
}
func (s *IoNet) TestConnection(ctx context.Context, key string) (any, error) {
	if key == "" {
		_, stored, err := s.providerKey(ctx)
		if err != nil {
			return nil, err
		}
		key = stored
	}
	raw, _, err := s.request(ctx, key, http.MethodGet, "/hardware", nil)
	if err != nil {
		return nil, err
	}
	value, err := decodeIoNet(raw)
	if err != nil {
		return nil, err
	}
	// Authentication errors wrapped in HTTP 200 are not connection success.
	switch v := value.(type) {
	case map[string]any:
		if _, ok := v["hardware"].([]any); !ok {
			return nil, ErrIoNetUpstream
		}
	case []any:
	default:
		return nil, ErrIoNetUpstream
	}
	return redactIoNet(value, key), nil
}

// The current provider logs endpoint is SSE even for historical log reads.
// Sample complete lines for the existing explicit-refresh viewer, then close
// upstream; do not wait forever for EOF or silently turn a stream into JSON.
func (s *IoNet) Logs(ctx context.Context, deployment, container, stream string, limit int) (string, error) {
	if !ioNetID.MatchString(deployment) || !ioNetID.MatchString(container) || (stream != "stdout" && stream != "stderr" && stream != "all") || limit < 1 || limit > 500 {
		return "", ErrInvalid
	}
	_, key, err := s.providerKey(ctx)
	if err != nil {
		return "", err
	}
	if stream == "all" {
		stdout, err := s.sampleLogs(ctx, key, deployment, container, "stdout", limit)
		if err != nil {
			return "", err
		}
		stderr, err := s.sampleLogs(ctx, key, deployment, container, "stderr", limit)
		if err != nil {
			return "", err
		}
		return "stdout:\n" + stdout + "\nstderr:\n" + stderr, nil
	}
	return s.sampleLogs(ctx, key, deployment, container, stream, limit)
}
func (s *IoNet) sampleLogs(ctx context.Context, key, deployment, container, stream string, limit int) (string, error) {
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, ioNetOrigin+"/deployment/"+deployment+"/log/"+container+"?stream="+url.QueryEscape(stream), nil)
	if err != nil {
		return "", ErrInvalid
	}
	req.Header.Set("x-api-key", key)
	req.Header.Set("Accept", "text/event-stream")
	client := *s.client
	client.Jar = nil
	client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	response, err := client.Do(req)
	if err != nil {
		return "", ErrIoNetUpstream
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return "", ErrIoNetUpstream
	}
	scanner := bufio.NewScanner(io.LimitReader(response.Body, ioNetResponseLimit))
	scanner.Buffer(make([]byte, 4096), 64<<10)
	var lines []string
	for scanner.Scan() {
		line := scanner.Text()
		if strings.HasPrefix(line, ":") || strings.HasPrefix(line, "event:") || strings.HasPrefix(line, "id:") || line == "" {
			continue
		}
		line = strings.TrimPrefix(strings.TrimPrefix(line, "data:"), " ")
		lines = append(lines, strings.ReplaceAll(line, key, "[redacted]"))
		if len(lines) >= limit {
			break
		}
	}
	if scanner.Err() != nil && len(lines) == 0 {
		return "", ErrIoNetUpstream
	}
	return strings.Join(lines, "\n"), nil
}
