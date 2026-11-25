// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later
package assistant_services

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/adminskillfacts"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// This adapter borrows the service transaction and reads only Go-owned skill
// content. A fresh Core observation is required before any target-row query.
func (s *Service) adminSkillsRead(ctx context.Context, tx *sql.Tx, in assistant.AgentInvocation, actor access.Principal) (adminSkillProjection, error) {
	input, err := parseAdminSkillArguments(in.Call.Arguments)
	if err != nil {
		return adminSkillProjection{}, err
	}
	authority, err := s.adminSkillAuthority(ctx, in.Credential, actor.UserID, input.TargetUserID)
	if err != nil {
		return adminSkillProjection{}, err
	}
	var profile adminSkillProfile
	var tags []byte
	err = tx.QueryRowContext(ctx, `SELECT CASE WHEN octet_length(profile_key)<=65536 THEN profile_key ELSE NULL END,
        CASE WHEN octet_length(tags::text)<=65536 THEN tags ELSE NULL END,
        strategy,source,enabled,updated_by,created_at,updated_at
        FROM lmm_assistant.profiles WHERE user_id=$1`, input.TargetUserID).
		Scan(&profile.ProfileKey, &tags, &profile.Strategy, &profile.Source, &profile.Enabled, &profile.UpdatedBy, &profile.CreatedAt, &profile.UpdatedAt)
	var view *adminSkillProfile
	if err == nil {
		if len(tags) == 0 || bytes.Equal(bytes.TrimSpace(tags), []byte("null")) || json.Unmarshal(tags, &profile.Tags) != nil {
			return adminSkillProjection{}, access.ErrUnavailable
		}
		view, err = normalizeAdminSkillProfile(profile)
		if err != nil {
			return adminSkillProjection{}, err
		}
	} else if !errors.Is(err, sql.ErrNoRows) {
		return adminSkillProjection{}, access.ErrUnavailable
	}
	rows, err := tx.QueryContext(ctx, `SELECT id,title,content,CASE WHEN octet_length(tags::text)<=65536 THEN tags ELSE NULL END,source,enabled,created_at,updated_at
        FROM lmm_assistant.memories WHERE user_id=$1 AND ($2::bigint=0 OR id<$2)
        ORDER BY id DESC LIMIT $3`, input.TargetUserID, input.BeforeID, input.Limit+1)
	if err != nil {
		return adminSkillProjection{}, access.ErrUnavailable
	}
	defer rows.Close()
	memories := make([]Memory, 0, input.Limit+1)
	for rows.Next() {
		memory, err := scanMemory(rows)
		if err != nil || memory.ID <= 0 || memory.ID > adminskillfacts.MaxSafeInteger ||
			(memory.Source != "assistant" && memory.Source != "administrator") {
			return adminSkillProjection{}, access.ErrUnavailable
		}
		memories = append(memories, memory)
	}
	if rows.Err() != nil {
		return adminSkillProjection{}, access.ErrUnavailable
	}
	return boundedAdminSkillProjection(authority, view, memories, input.Limit)
}
