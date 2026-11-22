package assistant_services

import (
	"context"
	"encoding/json"
	"testing"
)

// These stubs exercise only discovery; no call produces a confirmation proof,
// domain write, eligibility assertion, or a successful tool execution.
type restoredDiscoveryPorts struct{}

func (restoredDiscoveryPorts) ServiceFacts(context.Context, string) (json.RawMessage, error) {
	return nil, nil
}
func (restoredDiscoveryPorts) SetupGuide(context.Context, string, json.RawMessage) (json.RawMessage, error) {
	return nil, nil
}
func (restoredDiscoveryPorts) Navigation(context.Context, string, NavigationInput) (json.RawMessage, error) {
	return nil, nil
}
func (restoredDiscoveryPorts) PublishedPolicy(context.Context, string, string, string) (PublishedPolicy, error) {
	return PublishedPolicy{}, nil
}
func (restoredDiscoveryPorts) SupportEligibility(context.Context, string) (Eligibility, error) {
	return Eligibility{}, nil
}
func (restoredDiscoveryPorts) Prepare(context.Context, string, string, string, string, json.RawMessage) (Challenge, error) {
	return Challenge{}, nil
}
func (restoredDiscoveryPorts) Verify(context.Context, string, string, string, string) (BrowserProof, error) {
	return BrowserProof{}, nil
}

func TestRestoredBindingsKeepMissingDependencyAndHumanProofGates(t *testing.T) {
	p := restoredDiscoveryPorts{}
	configured := Dependencies{Public: p, Navigation: p, Policies: p, SupportEligibility: p, Confirmations: p}
	for _, test := range []struct {
		name   string
		remove func(*Dependencies)
		names  []string
	}{
		{"public", func(d *Dependencies) { d.Public = nil }, []string{"get_service_facts", "get_setup_guide"}},
		{"navigation", func(d *Dependencies) { d.Navigation = nil }, []string{"navigate_to_page"}},
		{"policy", func(d *Dependencies) { d.Policies = nil }, []string{"get_site_policy", "search_site_policies"}},
		{"support eligibility", func(d *Dependencies) { d.SupportEligibility = nil }, []string{"get_human_support_status", "book_technical_support"}},
		{"browser proof", func(d *Dependencies) { d.Confirmations = nil }, []string{"create_site_issue", "update_site_issue", "request_human_support"}},
	} {
		t.Run(test.name, func(t *testing.T) {
			d := configured
			before := (&Service{dependencies: d}).AvailableBindings()
			test.remove(&d)
			after := (&Service{dependencies: d}).AvailableBindings()
			for _, name := range test.names {
				foundBefore, foundAfter := false, false
				for _, b := range before {
					foundBefore = foundBefore || b.Name == name
				}
				for _, b := range after {
					foundAfter = foundAfter || b.Name == name
				}
				if !foundBefore || foundAfter {
					t.Fatal("dependency gate changed", name)
				}
			}
			for _, b := range after {
				if b.Name == "prepare_admin_site_policy_change" {
					t.Fatal("missing policy-change authority advertised")
				}
			}
		})
	}
}
