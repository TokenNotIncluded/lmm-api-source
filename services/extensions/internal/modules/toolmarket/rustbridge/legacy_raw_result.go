// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"

	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

// Only the closed terminal-proof leaves accepts a 2 MiB original MCP result encoded
// as base64. Native v1 and ordinary v2 request limits remain unchanged.
func (a *Adapter) requestLegacyResult(ctx context.Context, action string, in, out any) error {
	if action != "calls/settle" && action != "calls/release" && action != "fixed-witness/settle" && action != "fixed-witness/release" {
		return market.ErrInvalid
	}
	return a.requestWithSpec(ctx, requestSpec{major: 2, action: action, requestBytes: 2850000, responseBytes: 64 << 10}, "", in, out)
}
