// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"time"

	"github.com/lib/pq"
)

type IoNetOperation struct {
	Key            string          `json:"operation_key"`
	Kind           string          `json:"kind"`
	State          string          `json:"state"`
	DeploymentID   string          `json:"deployment_id,omitempty"`
	ConfigRevision int64           `json:"config_revision"`
	Result         json.RawMessage `json:"result,omitempty"`
}
type ioNetIntent struct {
	APIKey   string          `json:"api_key"`
	Method   string          `json:"method"`
	Endpoint string          `json:"endpoint"`
	Body     json.RawMessage `json:"body,omitempty"`
}

func ioNetPurpose(actor int64, key string) string {
	return fmt.Sprintf("lmm_operations.ionet.operation.v1:%d:%s", actor, key)
}

func (s *IoNet) Operation(ctx context.Context, actor int64, key string) (IoNetOperation, error) {
	out, _, _, err := s.operation(ctx, actor, key)
	return out, err
}
func (s *IoNet) operation(ctx context.Context, actor int64, key string) (IoNetOperation, string, string, error) {
	if s == nil || s.db == nil {
		return IoNetOperation{}, "", "", ErrUnavailable
	}
	if !validID(actor) || !ioNetOperationKey.MatchString(key) {
		return IoNetOperation{}, "", "", ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	out := IoNetOperation{Key: key}
	var sealed, digest string
	var result []byte
	err := s.db.QueryRowContext(ctx, `SELECT kind,state,deployment_id,config_revision,result,request_ciphertext,request_sha256 FROM lmm_operations.ionet_operations WHERE actor_user_id=$1 AND operation_key=$2`, actor, key).Scan(&out.Kind, &out.State, &out.DeploymentID, &out.ConfigRevision, &result, &sealed, &digest)
	if errors.Is(err, sql.ErrNoRows) {
		return out, "", "", ErrNotFound
	}
	if err != nil {
		return out, "", "", ErrUnavailable
	}
	out.Result = result
	return out, sealed, digest, nil
}

// Mutation dispatches only the winning INSERT. Duplicate keys read the original
// receipt; an unknown result is never automatically dispatched a second time.
func (s *IoNet) Mutation(ctx context.Context, actor int64, key, kind, deployment, resource, method, endpoint string, body json.RawMessage) (IoNetOperation, error) {
	if !validID(actor) || !ioNetOperationKey.MatchString(key) {
		return IoNetOperation{}, ErrInvalid
	}
	digestBytes := sha256.Sum256(append([]byte(kind+"\n"+deployment+"\n"), body...))
	digest := hex.EncodeToString(digestBytes[:])
	prior, _, original, err := s.operation(ctx, actor, key)
	if err == nil {
		if original != digest {
			return IoNetOperation{}, ErrConflict
		}
		return prior, nil
	}
	if !errors.Is(err, ErrNotFound) {
		return IoNetOperation{}, err
	}
	settings, apiKey, err := s.providerKey(ctx)
	if err != nil {
		return IoNetOperation{}, err
	}
	intent, err := json.Marshal(ioNetIntent{APIKey: apiKey, Method: method, Endpoint: endpoint, Body: body})
	if err != nil {
		return IoNetOperation{}, ErrInvalid
	}
	sealed, err := sealCredentialPurpose(s.credentials, ioNetPurpose(actor, key), intent)
	if err != nil {
		return IoNetOperation{}, err
	}
	writeCtx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var inserted string
	err = s.db.QueryRowContext(writeCtx, `INSERT INTO lmm_operations.ionet_operations(actor_user_id,operation_key,kind,resource_key,deployment_id,request_sha256,config_revision,state,request_ciphertext)
 SELECT $1,$2,$3,$4,$5,$6,revision,'unknown',$7 FROM lmm_operations.ionet_config WHERE singleton AND revision=$8 AND enabled
 ON CONFLICT(actor_user_id,operation_key) DO NOTHING RETURNING operation_key`, actor, key, kind, resource, deployment, digest, sealed, settings.Revision).Scan(&inserted)
	if errors.Is(err, sql.ErrNoRows) {
		prior, _, original, e := s.operation(ctx, actor, key)
		if e == nil && original == digest {
			return prior, nil
		}
		return IoNetOperation{}, ErrConflict
	}
	if err != nil {
		var p *pq.Error
		if errors.As(err, &p) && p.Code == "23505" {
			return IoNetOperation{}, ErrConflict
		}
		return IoNetOperation{}, ErrUnavailable
	}
	out := IoNetOperation{Key: key, Kind: kind, State: "unknown", DeploymentID: deployment, ConfigRevision: settings.Revision}
	// Finish recording the already-admitted original operation even if the Web
	// connection closes. Work is bounded; process death leaves the durable fence.
	runCtx, stop := context.WithTimeout(context.WithoutCancel(ctx), 25*time.Second)
	defer stop()
	raw, status, requestErr := s.request(runCtx, apiKey, method, endpoint, body)
	if requestErr != nil {
		// Only explicit validation/auth rejection proves no remote mutation. 5xx,
		// network loss, conflicts and ambiguous replies preserve UNKNOWN.
		if status == 400 || status == 401 || status == 403 || status == 422 {
			out.State = "rejected"
		} else {
			return out, nil
		}
	} else {
		value, e := decodeIoNet(raw)
		if e != nil {
			return out, nil
		}
		object, ok := value.(map[string]any)
		if !ok {
			return out, nil
		}
		if kind == "create" {
			id, _ := object["deployment_id"].(string)
			if !ioNetID.MatchString(id) {
				return out, nil
			}
			out.DeploymentID = id
		}
		if kind == "rename" {
			if status, _ := object["status"].(string); status != "success" {
				return out, nil
			}
		}
		if kind != "create" && kind != "rename" {
			responseID, _ := object["deployment_id"].(string)
			if responseID == "" {
				responseID, _ = object["id"].(string)
			}
			if responseID != deployment {
				return out, nil
			}
		}
		out.State = "succeeded"
		out.Result, _ = json.Marshal(redactIoNet(value, apiKey))
	}
	if _, err := s.db.ExecContext(runCtx, `UPDATE lmm_operations.ionet_operations SET state=$1,deployment_id=$2,result=$3,updated_at=clock_timestamp() WHERE actor_user_id=$4 AND operation_key=$5 AND state='unknown'`, out.State, out.DeploymentID, nullableIoNetResult(out.Result), actor, key); err != nil {
		out.State = "unknown"
		out.Result = nil
	}
	return out, nil
}
func nullableIoNetResult(raw json.RawMessage) any {
	if len(raw) == 0 {
		return nil
	}
	return string(raw)
}

// Recover handles even if the browser lost local storage, without exposing
// another administrator's operation or any encrypted request snapshot.
func (s *IoNet) Pending(ctx context.Context, actor int64) ([]IoNetOperation, error) {
	if s == nil || s.db == nil {
		return nil, ErrUnavailable
	}
	if !validID(actor) {
		return nil, ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	rows, err := s.db.QueryContext(ctx, `SELECT operation_key,kind,state,deployment_id,config_revision FROM lmm_operations.ionet_operations WHERE actor_user_id=$1 AND state='unknown' ORDER BY created_at DESC LIMIT 100`, actor)
	if err != nil {
		return nil, ErrUnavailable
	}
	defer rows.Close()
	out := make([]IoNetOperation, 0)
	for rows.Next() {
		var op IoNetOperation
		if rows.Scan(&op.Key, &op.Kind, &op.State, &op.DeploymentID, &op.ConfigRevision) != nil {
			return nil, ErrUnavailable
		}
		out = append(out, op)
	}
	if rows.Err() != nil {
		return nil, ErrUnavailable
	}
	return out, nil
}

// Recovery is deliberately read-only upstream. Receipt-known success survives
// a lost Web response; a provider timeout cannot be converted into success by
// observing a deployment's existence or a changed remaining-duration counter.
func (s *IoNet) Recover(ctx context.Context, actor int64, key string) (IoNetOperation, error) {
	out, sealed, _, err := s.operation(ctx, actor, key)
	if err != nil || out.State != "unknown" {
		return out, err
	}
	plain, err := openCredentialPurpose(s.credentials, ioNetPurpose(actor, key), sealed)
	if err != nil {
		return out, err
	}
	var intent ioNetIntent
	if json.Unmarshal(plain, &intent) != nil || !validRepositoryToken(intent.APIKey) {
		return out, ErrUnavailable
	}
	if out.DeploymentID == "" {
		return out, nil
	}
	_, _, _ = s.request(ctx, intent.APIKey, http.MethodGet, "/deployment/"+out.DeploymentID, nil)
	return out, nil
}

// CancelMissing persists a fence rather than treating a momentary 404 as proof
// that the original POST stopped. The PK arbitrates a concurrent late INSERT.
// Already-dispatched UNKNOWN work cannot be cancelled or declared unbilled here.
func (s *IoNet) CancelMissing(ctx context.Context, actor int64, key string) (IoNetOperation, error) {
	if s == nil || s.db == nil {
		return IoNetOperation{}, ErrUnavailable
	}
	if !validID(actor) || !ioNetOperationKey.MatchString(key) {
		return IoNetOperation{}, ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	digest := sha256.Sum256([]byte("cancel-missing"))
	_, err := s.db.ExecContext(ctx, `INSERT INTO lmm_operations.ionet_operations(actor_user_id,operation_key,kind,resource_key,request_sha256,config_revision,state,request_ciphertext)
 VALUES($1,$2,'cancel',$3,$4,0,'cancelled','') ON CONFLICT(actor_user_id,operation_key) DO NOTHING`, actor, key, "cancel:"+key, hex.EncodeToString(digest[:]))
	if err != nil {
		return IoNetOperation{}, ErrUnavailable
	}
	return s.Operation(ctx, actor, key)
}
