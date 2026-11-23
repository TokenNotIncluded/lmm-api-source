package assistant_catalog

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/bounties"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
)

const catalogueSession = "synthetic-original-session-aaaaaaaaaaaaaaaa"

type liveAuthority struct {
	calls   int
	revoked bool
}

func (a *liveAuthority) Check(_ context.Context, token string, account access.Account, permission access.Permission) (access.Principal, error) {
	a.calls++
	if token != catalogueSession || a.revoked {
		return access.Principal{}, access.ErrUnauthorized
	}
	if account != (access.Account{}) || permission != access.Read {
		return access.Principal{}, access.ErrForbidden
	}
	// Level zero has no developer qualification or management privilege.
	return access.Principal{UserID: 7, Level: 0, Account: access.Account{Kind: "personal", ID: 7}}, nil
}

type catalogueProbe struct {
	rows    []map[string]any
	row     map[string]any
	err     error
	calls   int
	query   store.CatalogueQuery
	viewer  int64
	token   string
	product string
	after   func()
}

func (p *catalogueProbe) CatalogueForViewer(_ context.Context, token string, viewer int64, query store.CatalogueQuery) ([]map[string]any, error) {
	p.calls++
	p.token = token
	p.viewer = viewer
	p.query = query
	if p.after != nil {
		p.after()
	}
	return p.rows, p.err
}
func (p *catalogueProbe) ProductForViewer(_ context.Context, token string, viewer int64, id string) (map[string]any, error) {
	p.calls++
	p.token = token
	p.viewer = viewer
	p.product = id
	if p.after != nil {
		p.after()
	}
	return p.row, p.err
}

type bountyProbe struct {
	fee            bounties.FeeConfig
	board, detail  map[string]any
	result         bounties.Result
	err            error
	calls          int
	token, project string
	page, size     int
	invocation     bounties.Invocation
	disputes       []map[string]any
	administrative bool
	status         string
	limit          int
}

func (p *bountyProbe) Config(context.Context) (bounties.FeeConfig, error) {
	p.calls++
	return p.fee, p.err
}
func (p *bountyProbe) BoardFor(_ context.Context, token string, page, size int) (map[string]any, error) {
	p.calls++
	p.token = token
	p.page = page
	p.size = size
	return p.board, p.err
}
func (p *bountyProbe) Detail(_ context.Context, token, id string) (map[string]any, error) {
	p.calls++
	p.token = token
	p.project = id
	return p.detail, p.err
}
func (p *bountyProbe) BuiltinInvoke(_ context.Context, in bounties.Invocation) (bounties.Result, error) {
	p.calls++
	p.invocation = in
	return p.result, p.err
}

func (p *bountyProbe) DisputesForViewer(_ context.Context, token string, administrative bool, status string, limit int) ([]map[string]any, error) {
	p.calls++
	p.token, p.administrative, p.status, p.limit = token, administrative, status, limit
	return p.disputes, p.err
}

func fixture(t *testing.T) (*Executor, *liveAuthority, *catalogueProbe, *bountyProbe) {
	t.Helper()
	a := &liveAuthority{}
	e, err := New(a, nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	s := &catalogueProbe{}
	b := &bountyProbe{}
	e.store = s
	e.bounties = b
	return e, a, s, b
}
func invocation(t *testing.T, e *Executor, name, args string) (*tool, assistant.AgentInvocation) {
	t.Helper()
	for _, b := range e.Bindings() {
		if b.Name == name {
			target := b.Executor.(*tool)
			in := assistant.AgentInvocation{Credential: catalogueSession, Actor: access.Principal{UserID: 7, Account: access.Account{Kind: "personal", ID: 7}}, Call: assistant.AgentToolCall{OperationID: "readonly-01", Scope: assistant.AgentToolScope{Name: name, Version: b.Version, Domain: "extension", SchemaSHA256: digest(b.Parameters), PriceVersion: b.PriceVersion, PriceSHA256: b.PriceSHA256}, Arguments: json.RawMessage(args), ArgumentsSHA256: digest([]byte(args))}}
			return target, in
		}
	}
	t.Fatal("missing binding", name)
	return nil, assistant.AgentInvocation{}
}
func product(id string) map[string]any {
	return map[string]any{"id": id, "title": "Actual catalogue title", "description": "Seller text", "status": "published", "record_kind": "legacy", "visibility": "public", "price_quota": int64(500000), "available_stock": int64(3), "payment_methods": []string{"wallet"}, "trading_paused": false, "variants": []any{map[string]any{"id": "sku-01", "name": "Custom size XL", "price_quota": int64(500000), "enabled": true}}}
}
func project(id string) map[string]any {
	return map[string]any{"id": id, "owner_user_id": 7, "title": "Actual bounty", "description": "Actual requirements", "rules": "Actual acceptance rules", "repository_url": "https://github.com/example/project", "reward_credit": int64(500000), "reward_slots": 1, "escrow_credit": int64(475000), "net_reward_credit": int64(475000), "status": "published"}
}
func raw(t *testing.T, value any) json.RawMessage {
	t.Helper()
	b, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func TestCatalogueBindingsAreExactAndOnlyInstalledDomainsAppear(t *testing.T) {
	e, _, _, _ := fixture(t)
	seen := map[string]bool{}
	for _, b := range e.Bindings() {
		if seen[b.Name] || b.Effect != "read_only" || b.Access != "user" || b.Version != assistant.FixedMainToolVersion || b.PriceSHA256 != assistant.FreeAgentPriceSHA256() {
			t.Fatal(b.Name)
		}
		seen[b.Name] = true
		var schema map[string]any
		if json.Unmarshal(b.Parameters, &schema) != nil || schema["additionalProperties"] != false {
			t.Fatal("unbounded schema", b.Name)
		}
	}
	if len(seen) != 4 {
		t.Fatal(seen)
	}
	e.store = nil
	if len(e.Bindings()) != 2 {
		t.Fatal("absent store advertised")
	}
	e.bounties = nil
	if len(e.Bindings()) != 0 {
		t.Fatal("absent bounty advertised")
	}
	if _, err := New(nil, nil, nil); !errors.Is(err, access.ErrInvalid) {
		t.Fatal(err)
	}
}

func TestCatalogueBindingsMatchReviewedSharedContractAndRegistry(t *testing.T) {
	e, _, s, _ := fixture(t)
	s.rows = []map[string]any{}
	bytes, err := os.ReadFile("../../../../../contracts/assistant-tools-v2.json")
	if err != nil {
		t.Fatal(err)
	}
	var doc struct {
		Tools []struct {
			Name, Description, Version, Domain string
			SchemaJSON                         string `json:"schema_json"`
			SchemaSHA256                       string `json:"schema_sha256"`
			PriceVersion                       string `json:"price_version"`
			PriceSHA256                        string `json:"price_sha256"`
		} `json:"tools"`
	}
	if json.Unmarshal(bytes, &doc) != nil {
		t.Fatal("invalid shared tool contract")
	}
	for _, b := range e.Bindings() {
		count := 0
		for _, row := range doc.Tools {
			if row.Name == b.Name {
				count++
				if row.Description != b.Description || row.Version != b.Version || row.Domain != "extension" || row.SchemaJSON != string(b.Parameters) || row.SchemaSHA256 != digest(b.Parameters) || row.PriceVersion != b.PriceVersion || row.PriceSHA256 != b.PriceSHA256 {
					t.Fatal("contract differs from executor", b.Name)
				}
			}
		}
		if count != 1 {
			t.Fatal("missing or duplicated contract", b.Name, count)
		}
	}
	registry, err := assistant.NewAgentRegistry(e.Bindings())
	if err != nil {
		t.Fatal(err)
	}
	_, in := invocation(t, e, "get_store_products", `{}`)
	in.Call.CoreConversationID = "lmm_cv_" + strings.Repeat("c", 64)
	in.Call.TurnID = "turn-1"
	in.Call.ModelOperationID = "model-1"
	in.Call.ProviderCallID = "call-1"
	if _, err = registry.Execute(context.Background(), in); err != nil {
		t.Fatal("real registry rejected installed read", err)
	}
	in.Call.Scope.PriceSHA256 = strings.Repeat("0", 64)
	if _, err = registry.Execute(context.Background(), in); !errors.Is(err, access.ErrConflict) || s.calls != 1 {
		t.Fatal("registry scope mismatch reached domain", err, s.calls)
	}
}

func TestCatalogueArgumentsCannotSelectActorWriteOrArbitraryOperation(t *testing.T) {
	cases := []struct{ name, args string }{
		{"get_store_products", `{"actor":8}`}, {"get_store_products", `{"query":null}`}, {"get_store_products", `{"offset":null}`}, {"get_store_products", `{"limit":null}`},
		{"get_store_products", `{"limit":21}`}, {"get_store_products", `{"offset":10001}`}, {"get_store_products", `{"limit":1.5}`}, {"get_store_products", `{"query":"a","query":"b"}`},
		{"get_store_product", `{"product_id":"../private"}`}, {"get_store_product", `{"product_id":"p","owner_preview":true}`}, {"get_store_product", `{"product_id":null}`},
		{"get_bounty_data", `{"view":"owned","admin":true}`},
		{"get_bounty_data", `{"view":"owned","action":"publish"}`}, {"get_bounty_data", `{"view":"accepted","participant_user_id":8}`},
		{"get_bounty_data", `{"view":"board","page_size":51}`}, {"get_bounty_data", `{"view":"detail","project_id":9007199254740993}`},
		{"get_bounty_data", `{"view":"detail","project_id":1.5}`}, {"get_bounty_data", `{"view":"detail","project_id":null}`},
		{"get_bounty_data", `{"view":"detail","project_id":1,"page":1}`}, {"get_bounty_data", `{"view":"owned","project_id":1}`},
		{"get_bounty_guide", `{"amount":1}`}, {"get_bounty_guide", `{} {}`}, {"get_bounty_guide", `null`},
	}
	for _, tc := range cases {
		t.Run(tc.name+tc.args, func(t *testing.T) {
			e, _, s, b := fixture(t)
			target, in := invocation(t, e, tc.name, tc.args)
			if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrInvalid) {
				t.Fatal(err)
			}
			if s.calls != 0 || b.calls != 0 {
				t.Fatal("invalid input reached domain")
			}
		})
	}
	for _, query := range []string{strings.Repeat("a", 121), strings.Repeat("中", 67)} {
		e, _, s, _ := fixture(t)
		target, in := invocation(t, e, "get_store_products", string(raw(t, map[string]string{"query": query})))
		if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrInvalid) || s.calls != 0 {
			t.Fatal(err, s.calls)
		}
	}
}

func TestCatalogueActorScopeAndMidReadRevocationAreChecked(t *testing.T) {
	for _, edit := range []func(*assistant.AgentInvocation){
		func(in *assistant.AgentInvocation) { in.Actor.UserID = 8 }, func(in *assistant.AgentInvocation) { in.Actor.Account.ID = 8 },
		func(in *assistant.AgentInvocation) { in.Credential = "host-token" }, func(in *assistant.AgentInvocation) { in.Call.Scope.ResourceID = "pretend-approved" },
		func(in *assistant.AgentInvocation) { in.Call.Scope.SchemaSHA256 = strings.Repeat("0", 64) }, func(in *assistant.AgentInvocation) { in.Call.Arguments = json.RawMessage(`{"limit":1}`) },
	} {
		e, _, s, _ := fixture(t)
		s.rows = []map[string]any{}
		target, in := invocation(t, e, "get_store_products", `{}`)
		edit(&in)
		if _, err := target.Execute(context.Background(), in); err == nil || s.calls != 0 {
			t.Fatal("forged invocation reached domain", err)
		}
	}
	e, a, s, _ := fixture(t)
	s.row = product("p")
	s.after = func() { a.revoked = true }
	target, in := invocation(t, e, "get_store_product", `{"product_id":"p"}`)
	if out, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrUnauthorized) || len(out.ResultJSON) != 0 || s.calls != 1 || a.calls != 2 {
		t.Fatal("revoked read released result", out, err)
	}
}

func TestCatalogueProductsPreserveExactUnitsUnknownStockAndSafeProjection(t *testing.T) {
	e, a, s, _ := fixture(t)
	s.rows = []map[string]any{product("123e4567-e89b-12d3-a456-426614174000")}
	s.rows[0]["review_note"] = "PRIVATE-REVIEW"
	s.rows[0]["delivery_cards"] = []string{"PRIVATE-CARD"}
	s.rows[0]["description"] = strings.Repeat("x", 1201)
	s.rows[0]["variants"].([]any)[0].(map[string]any)["fixed_content"] = "PRIVATE-DELIVERY"
	target, in := invocation(t, e, "get_store_products", `{"query":"  actual  ","offset":2,"limit":1}`)
	out, err := target.Execute(context.Background(), in)
	if err != nil {
		t.Fatal(err)
	}
	var result productsResult
	if json.Unmarshal(out.ResultJSON, &result) != nil || len(result.Items) != 1 {
		t.Fatal(string(out.ResultJSON))
	}
	v := result.Items[0]
	if a.calls != 2 || s.token != catalogueSession || s.viewer != 7 || s.query.Text != "actual" || s.query.Offset != 2 || s.query.Limit != 1 {
		t.Fatal("actual principal/query lost", s)
	}
	if !result.HasMore || !v.DescriptionTruncated || len(v.Description) != 1200 || v.PriceQuota == nil || *v.PriceQuota != 500000 || *v.CreditsPerUSD != 500000 || v.VariantStockKnown || len(v.Variants) != 1 || v.Variants[0].Name != "Custom size XL" || v.Variants[0].Tradable || v.Variants[0].InventoryAvailable != nil {
		t.Fatal(v)
	}
	for _, secret := range []string{"PRIVATE-REVIEW", "PRIVATE-CARD", "PRIVATE-DELIVERY", catalogueSession} {
		if strings.Contains(string(out.ResultJSON), secret) {
			t.Fatal("private field leaked", secret)
		}
	}
	if out.ResultSHA256 != digest(out.ResultJSON) || out.State != assistant.AgentToolSucceeded || out.DomainReceiptID != "read:readonly-01" {
		t.Fatal(out)
	}
	// Native external currency retains minor units, not invented CREDIT.
	s.row = product("native-product")
	s.row["record_kind"] = "native"
	s.row["price_quota"] = nil
	s.row["variants"] = []any{map[string]any{"id": "native-sku", "name": "Actual USD SKU", "enabled": true, "price": store.Price{Amount: 1234, Currency: "USD"}, "price_quota": nil}}
	target, in = invocation(t, e, "get_store_product", `{"product_id":"native-product"}`)
	out, err = target.Execute(context.Background(), in)
	if err != nil {
		t.Fatal(err)
	}
	var detail productResult
	if json.Unmarshal(out.ResultJSON, &detail) != nil {
		t.Fatal(string(out.ResultJSON))
	}
	if detail.Product.PriceQuota != nil || detail.Product.CreditsPerUSD != nil || detail.Product.PriceUnit != "native_variant_units" || detail.Product.Variants[0].Price.Amount != 1234 || detail.Product.Variants[0].Price.Currency != "USD" {
		t.Fatal(detail)
	}
}

func TestCatalogueReadErrorsAndInspectNeverSubstituteOrRetryWrites(t *testing.T) {
	e, _, s, _ := fixture(t)
	s.row = product("p")
	target, in := invocation(t, e, "get_store_product", `{"product_id":"p"}`)
	if _, err := target.Inspect(context.Background(), in); err != nil || s.calls != 1 {
		t.Fatal(err, s.calls)
	}
	s.row = product("different")
	if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrConflict) {
		t.Fatal(err)
	}
	s.err = errors.New("driver password synthetic-private")
	if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrUnavailable) || strings.Contains(err.Error(), "password") {
		t.Fatal(err)
	}
	s.err = store.ErrNotFound
	if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrNotFound) {
		t.Fatal(err)
	}
}

func TestCatalogueBountyGuideUsesExactLiveFee(t *testing.T) {
	e, _, _, b := fixture(t)
	b.fee = bounties.FeeConfig{Enabled: true, Code: "configured", RateBasisPoints: 375, RatePercent: json.Number("3.75"), Limits: bounties.FeeLimits{RewardSlotsMin: 1, RewardSlotsMax: 100, TipCreditMin: 1}}
	target, in := invocation(t, e, "get_bounty_guide", `{}`)
	out, err := target.Execute(context.Background(), in)
	if err != nil {
		t.Fatal(err)
	}
	var v bountyGuide
	if json.Unmarshal(out.ResultJSON, &v) != nil || v.PlatformFeeBasisPoints != 375 || v.PlatformFeePercent != "3.75" || !v.ReadOnly || !v.Enabled {
		t.Fatal(string(out.ResultJSON))
	}
	b.fee.RatePercent = "3.76"
	if _, err = target.Execute(context.Background(), in); !errors.Is(err, access.ErrUnavailable) {
		t.Fatal("inconsistent fee accepted", err)
	}
	b.fee.RateBasisPoints = 0
	b.fee.RatePercent = "0.00"
	b.fee.Enabled = false
	b.fee.Code = "policy_not_configured"
	out, err = target.Execute(context.Background(), in)
	if err != nil || !strings.Contains(string(out.ResultJSON), `"enabled":false`) {
		t.Fatal(err, string(out.ResultJSON))
	}
}

func TestCatalogueBountyReadsUseOriginalSessionAndLiteralOwnerScope(t *testing.T) {
	e, _, _, b := fixture(t)
	b.board = map[string]any{"items": []any{project("51")}, "page": 2, "page_size": 3, "total": 8}
	target, in := invocation(t, e, "get_bounty_data", `{"view":"board","page":2,"page_size":3}`)
	out, err := target.Execute(context.Background(), in)
	if err != nil || b.token != catalogueSession || b.page != 2 || b.size != 3 {
		t.Fatal(err, b)
	}
	var result bountyResult
	if json.Unmarshal(out.ResultJSON, &result) != nil || !result.HasMore || result.Board.Items[0].RewardQuota != 500000 {
		t.Fatal(string(out.ResultJSON))
	}
	b.detail = map[string]any{"project": project("51"), "challenges": []any{}, "ledger": []any{}}
	target, in = invocation(t, e, "get_bounty_data", `{"view":"detail","project_id":51}`)
	if _, err = target.Execute(context.Background(), in); err != nil || b.project != "51" {
		t.Fatal(err, b.project)
	}
	for _, view := range []string{"owned", "accepted"} {
		if view == "owned" {
			b.result = bounties.Result{Items: []json.RawMessage{raw(t, project("51"))}}
		} else {
			b.result = bounties.Result{Items: []json.RawMessage{raw(t, map[string]any{"id": "challenge-1", "project_id": "51", "participant_user_id": "7", "owner_user_id": "8", "status": "accepted", "reward_credit": 500000, "tip_credit": 0, "evidence": map[string]any{"confirmation": "PRIVATE-CONFIRMATION"}})}}
		}
		target, in = invocation(t, e, "get_bounty_data", `{"view":"`+view+`","page":2,"page_size":4}`)
		out, err = target.Execute(context.Background(), in)
		if err != nil {
			t.Fatal(view, err)
		}
		got := b.invocation
		if got.Actor.Token != catalogueSession || got.Actor.UserID != "" || got.Actor.Admin || got.Command != (bounties.Command{Action: "list_" + view}) || got.Draft != (bounties.Draft{}) || got.Preview || got.Page != 2 || got.PageSize != 4 || strings.Contains(string(out.ResultJSON), "PRIVATE-CONFIRMATION") {
			t.Fatal("operation broadened or private command leaked", got, string(out.ResultJSON))
		}
	}
	b.err = bounties.ErrForbidden
	if _, err = target.Execute(context.Background(), in); !errors.Is(err, access.ErrForbidden) {
		t.Fatal("current authority failure ignored", err)
	}
}

func TestCatalogueBountyPrivateRowsCannotBelongToOtherUser(t *testing.T) {
	e, _, _, b := fixture(t)
	wrong := project("p")
	wrong["owner_user_id"] = 8
	b.result.Items = []json.RawMessage{raw(t, wrong)}
	target, in := invocation(t, e, "get_bounty_data", `{"view":"owned"}`)
	if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrForbidden) {
		t.Fatal(err)
	}
	b.result.Items = []json.RawMessage{raw(t, map[string]any{"id": "c", "project_id": "p", "participant_user_id": 8, "reward_credit": 0, "tip_credit": 0})}
	target, in = invocation(t, e, "get_bounty_data", `{"view":"accepted"}`)
	if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrForbidden) {
		t.Fatal(err)
	}
	b.detail = map[string]any{"project": wrong, "challenges": []any{map[string]any{"id": "c", "project_id": "p", "participant_user_id": 8, "reward_credit": 0, "tip_credit": 0}}, "ledger": []any{}}
	target, in = invocation(t, e, "get_bounty_data", `{"view":"detail","project_id":"p"}`)
	if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrForbidden) {
		t.Fatal(err)
	}
	otherViewer := project("p")
	otherViewer["viewer_challenge"] = map[string]any{"id": "c", "project_id": "p", "participant_user_id": 8, "reward_credit": 0, "tip_credit": 0}
	b.board = map[string]any{"items": []any{otherViewer}, "page": 1, "page_size": 20, "total": 1}
	target, in = invocation(t, e, "get_bounty_data", `{"view":"board"}`)
	if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrForbidden) {
		t.Fatal(err)
	}
}

func TestCatalogueBountyRequiredAmountsCannotBecomeInventedZero(t *testing.T) {
	for _, kind := range []string{"project", "challenge", "ledger"} {
		base := map[string]any{"reward_credit": int64(0), "escrow_credit": int64(0), "net_reward_credit": int64(0), "tip_credit": int64(0), "quota": int64(0)}
		fields := map[string][]string{"project": {"reward_credit", "escrow_credit", "net_reward_credit"}, "challenge": {"reward_credit", "tip_credit"}, "ledger": {"quota"}}[kind]
		decode := func(v map[string]any) error {
			b := raw(t, v)
			switch kind {
			case "project":
				var row bountyProject
				return json.Unmarshal(b, &row)
			case "challenge":
				var row bountyChallenge
				return json.Unmarshal(b, &row)
			default:
				var row bountyLedger
				return json.Unmarshal(b, &row)
			}
		}
		if err := decode(base); err != nil {
			t.Fatal("explicit zero rejected", kind, err)
		}
		for _, field := range fields {
			for _, mode := range []string{"missing", "null", "fraction", "quoted"} {
				t.Run(kind+"/"+field+"/"+mode, func(t *testing.T) {
					row := map[string]any{}
					for k, v := range base {
						row[k] = v
					}
					switch mode {
					case "missing":
						delete(row, field)
					case "null":
						row[field] = nil
					case "fraction":
						row[field] = 0.5
					case "quoted":
						row[field] = "0"
					}
					if err := decode(row); err == nil {
						t.Fatal("missing or invalid amount accepted", kind, field, mode)
					}
				})
			}
		}
	}
	// Verify the same failure propagates from a real tool call, not only a DTO.
	e, _, _, b := fixture(t)
	incomplete := project("p")
	delete(incomplete, "escrow_credit")
	b.board = map[string]any{"items": []any{incomplete}, "page": 1, "page_size": 20, "total": 1}
	target, in := invocation(t, e, "get_bounty_data", `{"view":"board"}`)
	if out, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrUnavailable) || len(out.ResultJSON) != 0 {
		t.Fatal("incomplete domain record released", err, string(out.ResultJSON))
	}
}

func TestCatalogueNativeNegativeCurrencyPriceIsRejectedByDomainPriceCodec(t *testing.T) {
	for _, bad := range []any{store.Price{Amount: -1, Currency: "USD"}, map[string]any{"amount_minor": -1, "currency": "USD"}, map[string]any{"amount_minor": 0, "currency": ""}} {
		row := product("native-product")
		row["record_kind"] = "native"
		row["price_quota"] = nil
		row["variants"] = []any{map[string]any{"id": "sku", "name": "Actual variant", "enabled": true, "price": bad, "price_quota": nil}}
		if _, err := productProjection(row, true); !errors.Is(err, access.ErrUnavailable) {
			t.Fatal("invalid native price accepted", err)
		}
	}
}
