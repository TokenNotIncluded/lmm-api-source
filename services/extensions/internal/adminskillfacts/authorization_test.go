package adminskillfacts

import (
	"strings"
	"testing"
)

func TestAuthorizationRequiresCurrentAdminAndPermittedTarget(t *testing.T) {
	const now int64 = 1795400000
	original := Authorization{Version: 1, ActorUserID: 10, TargetUserID: 20, ActorAuthVersion: 3, TargetAuthVersion: 5, ActorRole: "admin", TargetRole: "user", AuthorizationBindingSHA256: strings.Repeat("a", 64), CheckedAtUnix: now}
	if !original.validAt(10, 20, now) {
		t.Fatal("valid current observation rejected")
	}
	roles := []string{"user", "admin", "superadmin", "unknown"}
	for _, actor := range roles {
		for _, target := range roles {
			a := original
			a.ActorRole, a.TargetRole = actor, target
			allowed := map[[2]string]bool{{"admin", "user"}: true, {"superadmin", "user"}: true, {"superadmin", "admin"}: true}
			want := allowed[[2]string{actor, target}]
			if a.validAt(10, 20, now) != want {
				t.Fatalf("role pair %s/%s", actor, target)
			}
		}
	}
	for _, mutate := range []func(*Authorization){
		func(a *Authorization) { a.ActorUserID = 20 },
		func(a *Authorization) { a.TargetUserID = 10 },
		func(a *Authorization) { a.ActorAuthVersion = 0 },
		func(a *Authorization) { a.TargetAuthVersion = 0 },
		func(a *Authorization) { a.Version = 2 },
		func(a *Authorization) { a.CheckedAtUnix = now - 31 },
		func(a *Authorization) { a.CheckedAtUnix = now + 6 },
		func(a *Authorization) { a.AuthorizationBindingSHA256 = strings.Repeat("A", 64) },
		func(a *Authorization) { a.AuthorizationBindingSHA256 = strings.Repeat("a", 63) },
	} {
		a := original
		mutate(&a)
		if a.validAt(10, 20, now) {
			t.Fatalf("invalid observation accepted: %+v", a)
		}
	}
	own := original
	own.TargetUserID, own.TargetAuthVersion, own.TargetRole = own.ActorUserID, own.ActorAuthVersion, own.ActorRole
	if !own.validAt(10, 10, now) {
		t.Fatal("current administrator self-read was lost")
	}
	own.ActorRole, own.TargetRole = "user", "user"
	if own.validAt(10, 10, now) {
		t.Fatal("self-read bypassed the current administrator gate")
	}
	own.ActorRole, own.TargetRole = "admin", "admin"
	own.TargetAuthVersion++
	if own.validAt(10, 10, now) {
		t.Fatal("inconsistent self authority accepted")
	}
	later := original
	later.CheckedAtUnix++
	if !original.SameAuthority(later) {
		t.Fatal("fresh timestamps must not break the stable binding")
	}
	later.TargetAuthVersion++
	if original.SameAuthority(later) {
		t.Fatal("changed target authority replayed")
	}
	later = original
	later.AuthorizationBindingSHA256 = strings.Repeat("b", 64)
	if original.SameAuthority(later) {
		t.Fatal("changed policy/credential binding replayed")
	}
}
