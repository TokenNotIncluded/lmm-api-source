package assistant_services

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/adminskillfacts"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

type adminSkillSessionAuthority struct{}

func (adminSkillSessionAuthority) Check(ctx context.Context, credential string, account access.Account, permission access.Permission) (access.Principal, error) {
	if credential == "session-admin-501-new" {
		credential = "session-admin-501"
	}
	return (pgServiceAuthority{}).Check(ctx, credential, account, permission)
}

// The database and service transactions are real. The typed Core authority
// observation is synthetic here; Core identity/MFA/policy checks have separate PG tests.
func TestAdminSkillRestrictedPGReadReplayAndRevocation(t *testing.T) {
	ctx := context.Background()
	owner := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_OWNER_FILE")
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	const target int64 = 33001
	if _, err := owner.Exec(`INSERT INTO lmm_assistant.profiles(user_id,profile_key,tags,strategy,source,enabled,updated_by,created_at,updated_at)
        VALUES($1,$2,'["project"]','Private profile contact owner@example.invalid','administrator',false,501,1,1)`, target, strings.Repeat("a", 75)+"@example.invalid"); err != nil {
		t.Fatal(err)
	}
	var ids []int64
	for i := 0; i < 5; i++ {
		var id int64
		if err := owner.QueryRow(`INSERT INTO lmm_assistant.memories(user_id,title,content,tags,source,enabled,updated_by,created_at,updated_at)
            VALUES($1,$2,$3,'["project"]','administrator',$4,501,1,1) RETURNING id`,
			target, fmt.Sprintf("skill %d", i), fmt.Sprintf("Private target item %d email owner@example.invalid", i), i%2 == 0).Scan(&id); err != nil {
			t.Fatal(err)
		}
		ids = append(ids, id)
	}
	fingerprint := func() string {
		t.Helper()
		var value string
		if err := runtime.QueryRow(`SELECT md5(
          coalesce((SELECT row_to_json(p)::text FROM lmm_assistant.profiles p WHERE user_id=$1),'') ||
          coalesce((SELECT string_agg(row_to_json(m)::text,E'\n' ORDER BY id) FROM lmm_assistant.memories m WHERE user_id=$1),'')
        )`, target).Scan(&value); err != nil {
			t.Fatal(err)
		}
		return value
	}
	initial := fingerprint()
	current := adminSkillObservation(501, target)
	denied := false
	factCalls := 0
	s, err := New(ctx, runtime, pgServiceAuthority{}, Dependencies{AdminSkills: adminSkillReaderFunc(func(_ context.Context, credential string, user int64) (adminskillfacts.Authorization, error) {
		factCalls++
		if credential != "session-admin-501" || user != target || denied {
			return adminskillfacts.Authorization{}, access.ErrForbidden
		}
		return current, nil
	})})
	if err != nil {
		t.Fatal(err)
	}
	in := pgServiceInvocation("session-admin-501", adminSkillsTool, fmt.Sprintf(`{"target_user_id":%d,"limit":2}`, target))
	first, err := s.Execute(ctx, in)
	if err != nil || first.State != assistant.AgentToolSucceeded || factCalls != 3 {
		t.Fatal(first, err, factCalls)
	}
	var firstView adminSkillProjection
	if !decodeAdminSkillProjection(first.ResultJSON, &firstView) || firstView.Profile == nil || firstView.Profile.Enabled || len(firstView.Memories) != 2 || firstView.Memories[0].ID != ids[4] || firstView.Memories[1].ID != ids[3] || firstView.Memories[1].Enabled || !firstView.MemoriesTruncated || firstView.NextBeforeID != ids[3] {
		t.Fatal("bounded page or disabled record lost", firstView)
	}
	if bytes.Contains(first.ResultJSON, []byte("owner@example.invalid")) || bytes.Contains(first.ResultJSON, []byte(strings.Repeat("a", 75))) || firstView.Profile.ProfileKey != "[REDACTED_EMAIL]" {
		t.Fatal("stored private credential-like text not redacted")
	}
	if fingerprint() != initial {
		t.Fatal("administrator getter mutated skills")
	}
	for _, read := range []func(context.Context, assistant.AgentInvocation) (assistant.AgentToolResult, error){s.Execute, s.Inspect} {
		got, err := read(ctx, in)
		if err != nil || !bytes.Equal(got.ResultJSON, first.ResultJSON) {
			t.Fatal("original result replay changed", err)
		}
	}
	if pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_operations WHERE operation_id=$1`, in.Call.OperationID) != 1 {
		t.Fatal("replay wrote another operation")
	}
	var received []int64
	for _, m := range firstView.Memories {
		received = append(received, m.ID)
	}
	cursor := firstView.NextBeforeID
	for cursor != 0 {
		next := pgServiceInvocation("session-admin-501", adminSkillsTool, fmt.Sprintf(`{"target_user_id":%d,"limit":2,"before_id":%d}`, target, cursor))
		out, err := s.Execute(ctx, next)
		var view adminSkillProjection
		if err != nil || !decodeAdminSkillProjection(out.ResultJSON, &view) || len(view.Memories) == 0 {
			t.Fatal("next page unavailable", err)
		}
		for _, m := range view.Memories {
			received = append(received, m.ID)
		}
		cursor = view.NextBeforeID
	}
	if len(received) != len(ids) {
		t.Fatal("pagination lost a record", received, ids)
	}
	for i, id := range received {
		if id != ids[len(ids)-1-i] {
			t.Fatal("pagination duplicated/skipped", received, ids)
		}
	}
	// Inspect on an untouched operation never dispatches and creates no result.
	untouched := pgServiceInvocation("session-admin-501", adminSkillsTool, fmt.Sprintf(`{"target_user_id":%d}`, target))
	unknown, err := s.Inspect(ctx, untouched)
	if err != nil || unknown.State != assistant.AgentToolUnknown || pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_operations WHERE operation_id=$1`, untouched.Call.OperationID) != 0 || fingerprint() != initial {
		t.Fatal("inspect dispatched/wrote", unknown, err)
	}
	for _, mutation := range []string{"revoked", "target_promoted", "policy_changed"} {
		before := current
		switch mutation {
		case "revoked":
			denied = true
		case "target_promoted":
			current.TargetRole = "admin"
			current.TargetAuthVersion++
		case "policy_changed":
			current.AuthorizationBindingSHA256 = strings.Repeat("b", 64)
		}
		for _, read := range []func(context.Context, assistant.AgentInvocation) (assistant.AgentToolResult, error){s.Execute, s.Inspect} {
			out, err := read(ctx, in)
			if err == nil || len(out.ResultJSON) != 0 {
				t.Fatal("revoked target replay leaked", mutation, err)
			}
		}
		current = before
		denied = false
	}
	// A different current Session for the same administrator cannot replay the original result.
	changed := in
	changed.Credential = "session-admin-501-new"
	changed.Actor, _ = (adminSkillSessionAuthority{}).Check(ctx, changed.Credential, access.Account{}, access.Read)
	another, err := New(ctx, runtime, adminSkillSessionAuthority{}, Dependencies{AdminSkills: adminSkillReaderFunc(func(_ context.Context, credential string, user int64) (adminskillfacts.Authorization, error) {
		if credential != "session-admin-501-new" {
			return adminskillfacts.Authorization{}, access.ErrForbidden
		}
		return adminSkillObservation(501, user), nil
	})})
	if err != nil {
		t.Fatal(err)
	}
	if out, err := another.Inspect(ctx, changed); !errors.Is(err, access.ErrConflict) || len(out.ResultJSON) != 0 {
		t.Fatal("cross-session private replay accepted", err)
	}
	if fingerprint() != initial {
		t.Fatal("failed replay mutated stored skills")
	}
}

func TestAdminSkillRestrictedPGBoundsBeforeReturningUnicodeAndSecrets(t *testing.T) {
	ctx := context.Background()
	owner := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_OWNER_FILE")
	runtime := pgServiceDB(t, "LMM_ASSISTANT_SERVICES_PG_RUNTIME_FILE")
	const target int64 = 33002
	if _, err := owner.Exec(`INSERT INTO lmm_assistant.profiles(user_id,profile_key,tags,strategy,source,enabled,updated_by,created_at,updated_at)
      VALUES($1,$2,'[]','bounded profile','administrator',true,501,1,1)`, target, strings.Repeat("界", 81)); err != nil {
		t.Fatal(err)
	}
	s, err := New(ctx, runtime, pgServiceAuthority{}, Dependencies{AdminSkills: adminSkillReaderFunc(func(_ context.Context, credential string, user int64) (adminskillfacts.Authorization, error) {
		if credential != "session-admin-501" || user != target {
			return adminskillfacts.Authorization{}, access.ErrForbidden
		}
		return adminSkillObservation(501, target), nil
	})})
	if err != nil {
		t.Fatal(err)
	}
	in := pgServiceInvocation("session-admin-501", adminSkillsTool, fmt.Sprintf(`{"target_user_id":%d}`, target))
	got, err := s.Execute(ctx, in)
	var view adminSkillProjection
	if err != nil || !decodeAdminSkillProjection(got.ResultJSON, &view) || view.Profile == nil || view.Profile.ProfileKey != strings.Repeat("界", 80) || len(view.Profile.TruncatedFields) != 1 || view.Profile.TruncatedFields[0] != "profile_key" {
		t.Fatal("Unicode projection damaged or truncated invisibly", err)
	}
	if _, err := owner.Exec(`UPDATE lmm_assistant.profiles SET profile_key=$2 WHERE user_id=$1`, target, strings.Repeat("界", 21846)); err != nil {
		t.Fatal(err)
	}
	oversized := pgServiceInvocation("session-admin-501", adminSkillsTool, fmt.Sprintf(`{"target_user_id":%d}`, target))
	rejected, err := s.Execute(ctx, oversized)
	if !errors.Is(err, access.ErrUnavailable) || len(rejected.ResultJSON) != 0 || pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_operations WHERE operation_id=$1`, oversized.Call.OperationID) != 0 {
		t.Fatal("oversized profile was partially leaked or journaled", err)
	}
	if _, err := owner.Exec(`UPDATE lmm_assistant.profiles SET profile_key='bounded' WHERE user_id=$1`, target); err != nil {
		t.Fatal(err)
	}
	tags, _ := json.Marshal([]string{strings.Repeat("tag", 23000)})
	if _, err := owner.Exec(`INSERT INTO lmm_assistant.memories(user_id,title,content,tags,source,enabled,updated_by,created_at,updated_at)
      VALUES($1,'oversized tags','bounded content',$2,'administrator',true,501,1,1)`, target, string(tags)); err != nil {
		t.Fatal(err)
	}
	badTags := pgServiceInvocation("session-admin-501", adminSkillsTool, fmt.Sprintf(`{"target_user_id":%d}`, target))
	rejected, err = s.Execute(ctx, badTags)
	if !errors.Is(err, access.ErrUnavailable) || len(rejected.ResultJSON) != 0 || pgServiceCount(t, runtime, `SELECT count(*) FROM lmm_assistant.service_operations WHERE operation_id=$1`, badTags.Call.OperationID) != 0 {
		t.Fatal("unbounded memory tag was returned or journaled", err)
	}
}
