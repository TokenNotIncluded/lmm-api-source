package rustbridge

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

func (a *Adapter) hostReady(ctx context.Context, path string, protocol int) ([]byte, error) {
	if a == nil || a.client == nil {
		return nil, market.ErrUnavailable
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	request, e := http.NewRequestWithContext(ctx, http.MethodPost, "http://core-readiness"+path, strings.NewReader("{}"))
	if e != nil {
		return nil, market.ErrInvalid
	}
	request.Header.Set("Authorization", "Bearer "+a.token)
	request.Header.Set("X-Lmm-Protocol", strconv.Itoa(protocol))
	request.Header.Set("Content-Type", "application/json")
	response, e := a.client.Do(request)
	if e != nil {
		return nil, market.ErrUnavailable
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return nil, market.ErrUnavailable
	}
	raw, e := io.ReadAll(io.LimitReader(response.Body, 4097))
	if e != nil || len(raw) > 4096 {
		return nil, market.ErrUnavailable
	}
	return raw, nil
}
func exactReadyFields(raw []byte, out any, names ...string) bool {
	if access.Decode(raw, out) != nil {
		return false
	}
	var fields map[string]json.RawMessage
	if json.Unmarshal(raw, &fields) != nil || len(fields) != len(names) {
		return false
	}
	for _, key := range names {
		if fields[key] == nil {
			return false
		}
	}
	return true
}

// This digest identifies the exact catalog before the optional relay tools.
const preRelayCatalogDigest = "8df36b1effa55c753d26fe14e4ccea6140d7244271f30610965d21503c1cf519"

func (a *Adapter) ReadyBuiltins(ctx context.Context) error {
	return a.readyBuiltinCatalog(ctx, false)
}

func (a *Adapter) ReadyExpandedBuiltins(ctx context.Context) error {
	return a.readyBuiltinCatalog(ctx, true)
}

func (a *Adapter) readyBuiltinCatalog(ctx context.Context, requireExpanded bool) error {
	raw, e := a.hostReady(ctx, "/internal/builtins/v1/ready", 1)
	if e != nil {
		return e
	}
	var ack struct {
		Protocol int    `json:"protocol"`
		Service  string `json:"service"`
		Ready    bool   `json:"ready"`
		Tools    int    `json:"tools"`
		Catalog  string `json:"catalog_digest"`
	}
	count := 0
	for _, definition := range market.CompiledBuiltinDefinitions() {
		count += len(definition.Tools)
	}
	if !exactReadyFields(raw, &ack, "protocol", "service", "ready", "tools", "catalog_digest") || ack.Protocol != 1 || ack.Service != "builtins" || !ack.Ready {
		return market.ErrUnavailable
	}
	if ack.Tools == count && ack.Catalog == market.CompiledBuiltinCatalogDigest() {
		return nil
	}
	if !requireExpanded && ack.Tools == 29 && ack.Catalog == preRelayCatalogDigest {
		return nil
	}
	return market.ErrUnavailable
}
func (d *DrawingAdapter) Ready(ctx context.Context) error {
	if d == nil {
		return market.ErrUnavailable
	}
	raw, e := d.bridge.hostReady(ctx, "/internal/models/images/v1/ready", 2)
	if e != nil {
		return e
	}
	var ack struct {
		Protocol int    `json:"protocol"`
		Service  string `json:"service"`
		Ready    bool   `json:"ready"`
	}
	if !exactReadyFields(raw, &ack, "protocol", "service", "ready") || ack.Protocol != 2 || ack.Service != "images" || !ack.Ready {
		return market.ErrUnavailable
	}
	return nil
}
