package coreclient

import (
	"context"
	"google.golang.org/grpc/metadata"
	"testing"
)

func TestCommerceHistoricalFlagKeepsOwnedCredentialsAndRejectsMixedVersions(t *testing.T) {
	c := &Client{authorization: "synthetic-service"}
	for _, flags := range [][]string{{"legacy-balance-v1"}, {"legacy-balance-v2"}, {"partial-v1", "legacy-balance-v1"}} {
		in := metadata.Pairs("authorization", "forged", "x-lmm-user-credential", "forged-user")
		in.Set("x-lmm-commerce-refund", flags...)
		out, _ := metadata.FromOutgoingContext(c.commerceContext(metadata.NewOutgoingContext(context.Background(), in), "synthetic-user"))
		if out.Get("authorization")[0] != "synthetic-service" || out.Get("x-lmm-user-credential")[0] != "synthetic-user" {
			t.Fatal("credentials replaced")
		}
		if (len(out.Get("x-lmm-commerce-refund")) == 1) != (len(flags) == 1 && flags[0] == "legacy-balance-v1") {
			t.Fatal(flags, out)
		}
	}
}
