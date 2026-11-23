package coreclient

import (
	"context"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/edgecontext"
	"google.golang.org/grpc/metadata"
)

// Only these Assistant calls can carry the edge assertion. Ordinary Core calls
// still replace all caller metadata, and Core checks this proof independently
// from the UDS service credential and original Session.
func (c *AssistantClient) edgeContext(ctx context.Context, user string) context.Context {
	out := c.rpc.context(ctx, user)
	if proof, client, ok := edgecontext.Assertion(ctx); ok {
		out = metadata.AppendToOutgoingContext(out, "x-lmm-edge-proof", proof, "x-lmm-edge-client-ip", client)
	}
	return out
}
