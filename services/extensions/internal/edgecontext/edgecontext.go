// Package edgecontext carries an unverified edge assertion from the loopback
// HTTP boundary to the narrow Assistant RPC. Core alone verifies its proof.
package edgecontext

import (
	"context"
	"net"
	"net/http"
	"net/netip"
	"strings"
)

const ProofHeader = "X-LMM-Edge-Proof"
const ClientHeader = "X-LMM-Edge-Client-IP"

type key struct{}
type assertion struct{ proof, client string }

// Capture strips the private headers before any downstream handler or provider
// sees them. Shape checks bound the carrier; they do not authorize the address.
func Capture(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var proof, client []string
		for name, values := range r.Header {
			switch {
			case strings.EqualFold(name, ProofHeader):
				proof = append(proof, values...)
				delete(r.Header, name)
			case strings.EqualFold(name, ClientHeader):
				client = append(client, values...)
				delete(r.Header, name)
			}
		}
		// Reset any earlier assertion: this listener's physical request is the source.
		ctx := context.WithValue(r.Context(), key{}, assertion{})
		host, _, peerErr := net.SplitHostPort(r.RemoteAddr)
		peer, addrErr := netip.ParseAddr(host)
		if peerErr == nil && addrErr == nil && peer.Unmap().IsLoopback() && len(proof) == 1 && len(client) == 1 && visibleProof(proof[0]) {
			ip, err := netip.ParseAddr(client[0])
			if err == nil && ip.Zone() == "" {
				ctx = context.WithValue(ctx, key{}, assertion{proof: proof[0], client: ip.Unmap().String()})
			}
		}
		next.ServeHTTP(w, r.WithContext(ctx))
	})
}

func visibleProof(value string) bool {
	if len(value) < 32 || len(value) > 256 {
		return false
	}
	for i := range value {
		if value[i] < 33 || value[i] > 126 {
			return false
		}
	}
	return true
}

// Assertion returns only the listener-captured carrier, never forwarding headers
// or caller-supplied gRPC metadata. The values must not be logged or returned.
func Assertion(ctx context.Context) (proof, client string, present bool) {
	value, ok := ctx.Value(key{}).(assertion)
	return value.proof, value.client, ok && value.proof != "" && value.client != ""
}
