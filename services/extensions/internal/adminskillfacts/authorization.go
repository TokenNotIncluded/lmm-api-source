// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
// Package adminskillfacts carries a fresh Core authorization observation.
// It is not a bearer grant; hosts recheck Core around every read and replay.
package adminskillfacts

import "time"

const Feature = "assistant.admin_user_skills.v1"
const MaxSafeInteger int64 = 9007199254740991

type Authorization struct {
	Version                    uint32 `json:"version"`
	ActorUserID                int64  `json:"actor_user_id"`
	TargetUserID               int64  `json:"target_user_id"`
	ActorAuthVersion           int64  `json:"actor_auth_version"`
	TargetAuthVersion          int64  `json:"target_auth_version"`
	ActorRole                  string `json:"actor_role"`
	TargetRole                 string `json:"target_role"`
	AuthorizationBindingSHA256 string `json:"authorization_binding_sha256"`
	CheckedAtUnix              int64  `json:"checked_at_unix"`
}

func validID(id int64) bool { return id > 0 && id <= MaxSafeInteger }

func (a Authorization) Valid(actor, target int64) bool {
	return a.validAt(actor, target, time.Now().Unix())
}

func (a Authorization) validAt(actor, target, now int64) bool {
	if a.Version != 1 || !validID(actor) || !validID(target) ||
		a.ActorUserID != actor || a.TargetUserID != target ||
		a.ActorAuthVersion <= 0 || a.TargetAuthVersion <= 0 ||
		now <= 0 || a.CheckedAtUnix < now-30 || a.CheckedAtUnix > now+5 ||
		len(a.AuthorizationBindingSHA256) != 64 {
		return false
	}
	for _, ch := range a.AuthorizationBindingSHA256 {
		if !((ch >= '0' && ch <= '9') || (ch >= 'a' && ch <= 'f')) {
			return false
		}
	}
	if actor == target {
		return a.ActorAuthVersion == a.TargetAuthVersion &&
			((a.ActorRole == "admin" && a.TargetRole == "admin") ||
				(a.ActorRole == "superadmin" && a.TargetRole == "superadmin"))
	}
	return (a.ActorRole == "admin" && a.TargetRole == "user") ||
		(a.ActorRole == "superadmin" && (a.TargetRole == "user" || a.TargetRole == "admin"))
}

// SameAuthority compares a stored result binding to a newly validated Core
// observation. Neither the old timestamp nor this comparison authorizes a read.
func (a Authorization) SameAuthority(b Authorization) bool {
	return a.Version == b.Version && a.ActorUserID == b.ActorUserID &&
		a.TargetUserID == b.TargetUserID && a.ActorAuthVersion == b.ActorAuthVersion &&
		a.TargetAuthVersion == b.TargetAuthVersion && a.ActorRole == b.ActorRole &&
		a.TargetRole == b.TargetRole && a.AuthorizationBindingSHA256 == b.AuthorizationBindingSHA256
}
