package assistant_services

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/adminskillfacts"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type adminSkillReaderFunc func(context.Context, string, int64) (adminskillfacts.Authorization, error)

func (f adminSkillReaderFunc) AdminSkillAuthorization(ctx context.Context, credential string, target int64) (adminskillfacts.Authorization, error) {
	return f(ctx, credential, target)
}
func adminSkillObservation(actor, target int64) adminskillfacts.Authorization {
	return adminskillfacts.Authorization{
		Version: 1, ActorUserID: actor, TargetUserID: target, ActorAuthVersion: 3, TargetAuthVersion: 5,
		ActorRole: "admin", TargetRole: "user", AuthorizationBindingSHA256: strings.Repeat("a", 64), CheckedAtUnix: time.Now().Unix(),
	}
}
func adminSkillSavedResult(t *testing.T, in assistant.AgentInvocation, a adminskillfacts.Authorization) assistant.AgentToolResult {
	t.Helper()
	projection, err := boundedAdminSkillProjection(a, nil, []Memory{{ID: 20, Title: "private skill", Content: "private target data", Tags: []string{}, Source: "assistant", Enabled: true}}, 16)
	if err != nil {
		t.Fatal(err)
	}
	out, err := result(in.Call.OperationID, assistant.AgentToolSucceeded, projection, "service:"+in.Call.OperationID)
	if err != nil {
		t.Fatal(err)
	}
	return out
}
func TestAdminSkillArgumentsPreserveLegacyTargetAndBoundedPagination(t *testing.T) {
	parsed, err := parseAdminSkillArguments(json.RawMessage(`{"target_user_id":202}`))
	if err != nil || parsed.TargetUserID != 202 || parsed.Limit != 16 || parsed.BeforeID != 0 {
		t.Fatal(parsed, err)
	}
	for _, raw := range []string{
		`{}`, `{"user_id":202}`, `{"target_user_id":0}`, `{"target_user_id":2147483648}`,
		`{"target_user_id":202,"target_user_id":203}`,
		`{"target_user_id":202,"limit":0}`, `{"target_user_id":202,"limit":null}`,
		`{"target_user_id":202,"limit":17}`, `{"target_user_id":202,"limit":1.5}`,
		`{"target_user_id":202,"before_id":null}`, `{"target_user_id":202,"before_id":-1}`, `{"target_user_id":202,"before_id":9007199254740992}`,
		`{"target_user_id":202,"admin":true}`,
	} {
		if _, err := parseAdminSkillArguments(json.RawMessage(raw)); !errors.Is(err, access.ErrInvalid) {
			t.Fatalf("accepted malformed arguments %s: %v", raw, err)
		}
	}
	parsed, err = parseAdminSkillArguments(json.RawMessage(`{"target_user_id":2147483647,"before_id":9007199254740991,"limit":1}`))
	if err != nil || parsed.Limit != 1 {
		t.Fatal(parsed, err)
	}
}
func TestAdminSkillReadRechecksAuthorityAndNeverLeaksOnError(t *testing.T) {
	ctx := context.Background()
	in := pgServiceInvocation("session-admin-501", adminSkillsTool, `{"target_user_id":202}`)
	original := adminSkillObservation(501, 202)
	for _, scenario := range []string{"success", "before_denied", "actor_substitution", "after_denied", "policy_changed", "target_generation_changed", "stored_binding_changed", "read_error", "unknown_error", "unknown"} {
		t.Run(scenario, func(t *testing.T) {
			factCalls, reads := 0, 0
			saved := adminSkillSavedResult(t, in, original)
			if scenario == "stored_binding_changed" {
				changed := original
				changed.AuthorizationBindingSHA256 = strings.Repeat("b", 64)
				saved = adminSkillSavedResult(t, in, changed)
			}
			s := &Service{auth: pgServiceAuthority{}, dependencies: Dependencies{AdminSkills: adminSkillReaderFunc(func(_ context.Context, credential string, target int64) (adminskillfacts.Authorization, error) {
				factCalls++
				if credential != in.Credential || target != 202 {
					t.Fatal("substituted original credential/target")
				}
				a := original
				if scenario == "before_denied" || (scenario == "after_denied" && factCalls == 2) {
					return adminskillfacts.Authorization{}, access.ErrForbidden
				}
				if scenario == "actor_substitution" {
					a.ActorUserID = 502
				}
				if factCalls == 2 && scenario == "policy_changed" {
					a.AuthorizationBindingSHA256 = strings.Repeat("c", 64)
				}
				if factCalls == 2 && scenario == "target_generation_changed" {
					a.TargetAuthVersion++
				}
				return a, nil
			})}}
			out, err := s.guardAdminSkillRead(ctx, in, func(_ context.Context, actual assistant.AgentInvocation) (assistant.AgentToolResult, error) {
				reads++
				if actual.Call.OperationID != in.Call.OperationID {
					t.Fatal("read rebound operation")
				}
				if scenario == "read_error" {
					return saved, access.ErrUnavailable
				}
				if scenario == "unknown_error" {
					return assistant.AgentToolResult{OperationID: in.Call.OperationID, State: assistant.AgentToolUnknown, ResultJSON: json.RawMessage(`{"secret":"private target data"}`)}, access.ErrUnavailable
				}
				if scenario == "unknown" {
					return unknownResult(in.Call.OperationID), nil
				}
				return saved, nil
			})
			if scenario == "success" {
				if err != nil || reads != 1 || factCalls != 2 || !bytes.Equal(out.ResultJSON, saved.ResultJSON) {
					t.Fatal("stable exact replay failed", err, reads, factCalls)
				}
				return
			}
			if bytes.Contains(out.ResultJSON, []byte("private")) {
				t.Fatal("private result escaped rejected authority or read error")
			}
			if scenario == "unknown" || scenario == "unknown_error" {
				if out.State != assistant.AgentToolUnknown || out.OperationID != in.Call.OperationID {
					t.Fatal("original unknown state lost", out, err)
				}
				if (scenario == "unknown_error") != (err != nil) {
					t.Fatal("unknown error outcome changed", err)
				}
				return
			}
			if err == nil || len(out.ResultJSON) != 0 {
				t.Fatal("rejected read returned success/private result", err)
			}
			if (scenario == "before_denied" || scenario == "actor_substitution") && reads != 0 {
				t.Fatal("data read before valid authority")
			}
		})
	}
}
func TestAdminSkillProfileRedactsThenBoundsWithVisibleTruncation(t *testing.T) {
	p := adminSkillProfile{ProfileKey: strings.Repeat("界", 81), Tags: []string{}, Strategy: "Authorization: Bearer synthetic-secret-token\n" + strings.Repeat("字", 4100), Source: "administrator", Enabled: false, UpdatedBy: 501}
	for i := 0; i < 17; i++ {
		p.Tags = append(p.Tags, strings.Repeat("标", 49))
	}
	out, err := normalizeAdminSkillProfile(p)
	if err != nil || out.Enabled || len([]rune(out.ProfileKey)) != 80 || len([]rune(out.Strategy)) > 4000 || len(out.Tags) != 16 {
		t.Fatal(out, err)
	}
	raw, _ := json.Marshal(out)
	if bytes.Contains(raw, []byte("synthetic-secret-token")) || len(out.TruncatedFields) != 3 {
		t.Fatal("redaction/truncation contract lost")
	}
	p.Source = "caller-authority"
	if _, err := normalizeAdminSkillProfile(p); !errors.Is(err, access.ErrUnavailable) {
		t.Fatal("unknown stored source accepted")
	}
}
func TestAdminSkillPageBudgetNeverSkipsUnreturnedMemories(t *testing.T) {
	a := adminSkillObservation(501, 202)
	p := &adminSkillProfile{ProfileKey: "profile", Strategy: strings.Repeat("界", 4000), Tags: []string{}, Source: "administrator", UpdatedBy: 501, TruncatedFields: []string{}}
	memories := make([]Memory, 17)
	for i := range memories {
		memories[i] = Memory{ID: int64(100 - i), Title: strings.Repeat("界", 80), Content: strings.Repeat("字", 800), Source: "assistant", Enabled: i%2 == 0, Tags: []string{}}
		for j := 0; j < 8; j++ {
			memories[i].Tags = append(memories[i].Tags, strings.Repeat("标", 48))
		}
	}
	remaining := memories
	var got []int64
	for len(remaining) > 0 {
		page, err := boundedAdminSkillProjection(a, p, remaining, 16)
		if err != nil || len(page.Memories) == 0 {
			t.Fatal(page, err)
		}
		raw, _ := json.Marshal(page)
		if len(raw) > maxAdminSkillResultBytes {
			t.Fatal("transport budget exceeded")
		}
		for _, m := range page.Memories {
			got = append(got, m.ID)
		}
		remaining = remaining[len(page.Memories):]
		if len(remaining) > 0 {
			if !page.MemoriesTruncated || page.NextBeforeID != got[len(got)-1] || remaining[0].ID >= page.NextBeforeID {
				t.Fatal("page omitted or skipped unseen record")
			}
		} else if page.MemoriesTruncated || page.NextBeforeID != 0 {
			t.Fatal("terminal page claims another cursor")
		}
	}
	if len(got) != 17 {
		t.Fatal(got)
	}
	for i, id := range got {
		if id != int64(100-i) {
			t.Fatal("duplicate/skipped memory", got)
		}
	}
	small, err := boundedAdminSkillProjection(a, nil, memories[:3], 2)
	if err != nil || len(small.Memories) != 2 || !small.MemoriesTruncated || small.NextBeforeID != 99 || small.Memories[1].Enabled {
		t.Fatal("limit/disabled record changed", small, err)
	}
}
func TestAdminSkillAvailabilityNeedsOnlyItsActualCorePort(t *testing.T) {
	before := (&Service{}).AvailableBindings()
	s := &Service{dependencies: Dependencies{AdminSkills: adminSkillReaderFunc(func(context.Context, string, int64) (adminskillfacts.Authorization, error) {
		return adminskillfacts.Authorization{}, fmt.Errorf("discovery must not execute")
	})}}
	after := s.AvailableBindings()
	if len(after) != len(before)+1 {
		t.Fatal("wrong availability change", len(before), len(after))
	}
	count := 0
	previous := map[string]bool{}
	for _, b := range before {
		previous[b.Name] = true
	}
	for _, b := range after {
		if b.Name == adminSkillsTool {
			count++
		} else if !previous[b.Name] {
			t.Fatal("administrator port activated another dependency")
		}
	}
	if count != 1 {
		t.Fatal("administrator read missing or duplicate")
	}
}

func TestAdminSkillLargeStoredProjectionKeepsStrictReplayBoundary(t *testing.T) {
	in := pgServiceInvocation("session-admin-501", adminSkillsTool, `{"target_user_id":202}`)
	a := adminSkillObservation(501, 202)
	p := &adminSkillProfile{ProfileKey: "large", Strategy: strings.Repeat("界", 4000), Tags: []string{}, Source: "administrator", UpdatedBy: 501, TruncatedFields: []string{}}
	memories := []Memory{{ID: 2, Title: "entry", Content: strings.Repeat("字", 800), Tags: []string{}, Source: "assistant", Enabled: true},
		{ID: 1, Title: "entry2", Content: strings.Repeat("字", 800), Tags: []string{}, Source: "assistant", Enabled: false}}
	view, err := boundedAdminSkillProjection(a, p, memories, 16)
	if err != nil {
		t.Fatal(err)
	}
	original, err := result(in.Call.OperationID, assistant.AgentToolSucceeded, view, "service:"+in.Call.OperationID)
	if err != nil || len(original.ResultJSON) <= 16<<10 {
		t.Fatal("large fixture missing", err, len(original.ResultJSON))
	}
	s := &Service{auth: pgServiceAuthority{}, dependencies: Dependencies{AdminSkills: adminSkillReaderFunc(func(context.Context, string, int64) (adminskillfacts.Authorization, error) { return a, nil })}}
	got, err := s.guardAdminSkillRead(context.Background(), in, func(context.Context, assistant.AgentInvocation) (assistant.AgentToolResult, error) {
		return original, nil
	})
	if err != nil || !bytes.Equal(got.ResultJSON, original.ResultJSON) {
		t.Fatal("bounded valid projection refused", err)
	}
	for _, raw := range [][]byte{
		bytes.Replace(original.ResultJSON, []byte(`"ok":true`), []byte(`"ok":true,"ok":true`), 1),
		bytes.Replace(original.ResultJSON, []byte(`"ok":true`), []byte(`"ok":true,"grant":true`), 1),
		bytes.Replace(original.ResultJSON, []byte(`"sensitive_values_redacted":true,`), nil, 1),
		append(append([]byte(nil), original.ResultJSON...), []byte("{}")...),
	} {
		var parsed adminSkillProjection
		if decodeAdminSkillProjection(raw, &parsed) {
			t.Fatal("noncanonical or incomplete projection accepted")
		}
	}
}
