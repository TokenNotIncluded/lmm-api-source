// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package coreclient

import (
	"context"
	"testing"

	"google.golang.org/grpc/metadata"
)

func TestCommercePartialFlagPreservesOnlyExplicitVersionAndOwnedCredentials(t *testing.T) {
	c := &Client{authorization: "synthetic-service"}
	for _, flags := range [][]string{{"partial-v1"}, {"partial-v2"}, {"partial-v1", "partial-v1"}, nil} {
		incoming := metadata.Pairs("authorization", "forged", "x-lmm-user-credential", "forged-user", "x-lmm-retry", "unsafe")
		incoming.Set("x-lmm-commerce-refund", flags...)
		ctx := metadata.NewOutgoingContext(context.Background(), incoming)
		out, _ := metadata.FromOutgoingContext(c.commerceContext(ctx, "synthetic-user"))
		if out.Get("authorization")[0] != "synthetic-service" || out.Get("x-lmm-user-credential")[0] != "synthetic-user" || len(out.Get("x-lmm-retry")) != 0 {
			t.Fatal("untrusted metadata survived", out)
		}
		expected := len(flags) == 1 && flags[0] == "partial-v1"
		if (len(out.Get("x-lmm-commerce-refund")) == 1) != expected {
			t.Fatal(flags, out)
		}
		plain, _ := metadata.FromOutgoingContext(c.context(ctx, "synthetic-user"))
		if len(plain.Get("x-lmm-commerce-refund")) != 0 {
			t.Fatal("commerce flag leaked into ordinary client context")
		}
	}
}
