// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/mail"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"
)

// The signal game grants no money, credits or other rewards; it only keeps
// public leaderboard records.

func signalHash(s string) string {
	d := sha256.Sum256([]byte(s))
	return hex.EncodeToString(d[:])
}

func signalErr(status int, message string) *apiError {
	return &apiError{status, "SIGNAL_GAME_REJECTED", message}
}

func validSignalActor(actor, modelID, harness, agent string) bool {
	return (actor == "human" && modelID == "" && harness == "" && agent == "") ||
		(actor == "ai" && strings.TrimSpace(modelID) != "" && len(modelID) <= 100 && strings.TrimSpace(harness) != "" && len(harness) <= 60 && strings.TrimSpace(agent) != "" && len(agent) <= 80)
}

func (h *handlers) signalDaily(w http.ResponseWriter, r *http.Request) {
	size, err := strconv.Atoi(r.URL.Query().Get("size"))
	if r.URL.Query().Get("size") == "" {
		size, err = 5, nil
	}
	if err != nil || !validSignalSize(size) {
		respond(w, nil, signalErr(400, "Invalid board size"))
		return
	}
	day := h.s.now().UTC().Format("2006-01-02")
	seed, _ := signalDailySeed(day, size)
	respond(w, map[string]any{"day": day, "seed": seed, "size": size, "rules_version": signalRulesVersion, "tiles": signalGenerate(seed, size).Tiles, "max_moves": signalMaxMoves(size)}, nil)
}

// POST /api/games/signal/attempts starts a public, anonymous timed challenge.
func (h *handlers) signalBegin(w http.ResponseWriter, r *http.Request) {
	var in struct {
		Size      int    `json:"size"`
		Actor     string `json:"actor"`
		ModelID   string `json:"model_id"`
		Harness   string `json:"harness"`
		AgentName string `json:"agent_name"`
	}
	if err := decode(w, r, 1024, &in, false); err != nil || !validSignalSize(in.Size) || !validSignalActor(in.Actor, in.ModelID, in.Harness, in.AgentName) {
		respond(w, nil, signalErr(400, "Invalid challenge participant"))
		return
	}
	raw := make([]byte, 32)
	if _, err := rand.Read(raw); err != nil {
		respond(w, nil, signalErr(503, "Challenge unavailable"))
		return
	}
	token := base64.RawURLEncoding.EncodeToString(raw)
	now := h.s.now()
	day := now.UTC().Format("2006-01-02")
	seed, _ := signalDailySeed(day, in.Size)
	err := h.s.tx(r.Context(), func(ctx context.Context, tx *sql.Tx) error {
		if _, e := tx.ExecContext(ctx, `DELETE FROM lmm_community.signal_attempts a WHERE finished_at=0 AND created_at<$1 AND NOT EXISTS (SELECT FROM lmm_community.signal_records r WHERE r.attempt_id=a.id)`, now.Add(-24*time.Hour).Unix()); e != nil {
			return e
		}
		_, e := tx.ExecContext(ctx, `INSERT INTO lmm_community.signal_attempts(token_hash,seed,size,day,actor,model_id,harness,agent_name,started_at,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)`,
			signalHash(token), int64(seed), in.Size, day, in.Actor, in.ModelID, in.Harness, in.AgentName, now.UnixMilli()+3000, now.Unix())
		return e
	})
	if err != nil {
		respond(w, nil, signalErr(503, "Challenge unavailable"))
		return
	}
	respond(w, map[string]any{"token": token, "seed": seed, "size": in.Size, "day": day, "rules_version": signalRulesVersion, "countdown_seconds": 3, "max_moves": signalMaxMoves(in.Size)}, nil)
}

type signalAttempt struct {
	id, startedAt, finishedAt, elapsedMs, createdAt  int64
	seed                                             uint32
	size, moves                                      int
	day, actor, modelID, harness, agentName, actions string
}

const attemptColumns = `id,seed,size,day,actor,model_id,harness,agent_name,started_at,finished_at,moves,elapsed_ms,actions,created_at`

func scanAttempt(row rowScanner) (signalAttempt, error) {
	var a signalAttempt
	var seed int64
	err := row.Scan(&a.id, &seed, &a.size, &a.day, &a.actor, &a.modelID, &a.harness, &a.agentName, &a.startedAt, &a.finishedAt, &a.moves, &a.elapsedMs, &a.actions, &a.createdAt)
	a.seed = uint32(seed)
	return a, err
}

var errSignalTiming = signalErr(409, "Challenge is expired, changed, or still counting down")

func checkFinishTime(a signalAttempt, nowMs int64) error {
	if a.finishedAt != 0 {
		return nil
	}
	if nowMs < a.startedAt || nowMs-a.startedAt > 24*60*60*1000 {
		return errSignalTiming
	}
	return nil
}

func (h *handlers) signalFinish(w http.ResponseWriter, r *http.Request) {
	var in struct {
		Token   string `json:"token"`
		Actions []int  `json:"actions"`
	}
	if err := decode(w, r, 512<<10, &in, false); err != nil || len(in.Token) != 43 {
		respond(w, nil, signalErr(400, "Invalid challenge result"))
		return
	}
	hash := signalHash(in.Token)
	a, err := scanAttempt(h.s.db.QueryRowContext(r.Context(), `SELECT `+attemptColumns+` FROM lmm_community.signal_attempts WHERE token_hash=$1`, hash))
	if errors.Is(err, sql.ErrNoRows) {
		respond(w, nil, signalErr(404, "Challenge not found"))
		return
	}
	if err != nil {
		respond(w, nil, signalErr(503, "Challenge unavailable"))
		return
	}
	if err = checkFinishTime(a, h.s.now().UnixMilli()); err != nil {
		respond(w, nil, err)
		return
	}
	if _, err = signalReplay(a.seed, a.size, in.Actions, false); err != nil {
		respond(w, nil, signalErr(400, "Challenge replay rejected"))
		return
	}
	actions, _ := json.Marshal(in.Actions)
	err = h.s.tx(r.Context(), func(ctx context.Context, tx *sql.Tx) error {
		var e error
		if a, e = scanAttempt(tx.QueryRowContext(ctx, `SELECT `+attemptColumns+` FROM lmm_community.signal_attempts WHERE token_hash=$1 FOR UPDATE`, hash)); e != nil {
			return e
		}
		if a.finishedAt != 0 {
			if a.actions != string(actions) {
				return errSignalTiming
			}
			return nil
		}
		nowMs := h.s.now().UnixMilli()
		if e = checkFinishTime(a, nowMs); e != nil {
			return e
		}
		a.finishedAt, a.elapsedMs, a.actions, a.moves = nowMs, nowMs-a.startedAt, string(actions), len(in.Actions)
		_, e = tx.ExecContext(ctx, `UPDATE lmm_community.signal_attempts SET finished_at=$2,elapsed_ms=$3,actions=$4,moves=$5 WHERE id=$1`, a.id, a.finishedAt, a.elapsedMs, a.actions, a.moves)
		return e
	})
	if err != nil {
		var api *apiError
		if !errors.As(err, &api) {
			err = errSignalTiming
		}
		respond(w, nil, err)
		return
	}
	respond(w, map[string]any{"moves": a.moves, "elapsed_ms": a.elapsedMs, "finished_at": a.finishedAt}, nil)
}

type signalRank struct {
	Player     int64  `json:"player"`
	Name       string `json:"name"`
	Actor      string `json:"actor"`
	ModelID    string `json:"model_id"`
	Harness    string `json:"harness"`
	AgentName  string `json:"agent_name"`
	Note       string `json:"note"`
	Moves      int    `json:"moves"`
	ElapsedMs  int64  `json:"elapsed_ms"`
	FinishedAt int64  `json:"finished_at"`
}

// Player names are snapshots (imported legacy names, or a neutral label for new
// records) because Core exposes no public display-name read to extensions.
func (h *handlers) signalLeaderboard(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	day := q.Get("day")
	if day == "" {
		day = h.s.now().UTC().Format("2006-01-02")
	}
	size := 5
	if raw := q.Get("size"); raw != "" {
		var err error
		if size, err = strconv.Atoi(raw); err != nil {
			respond(w, nil, signalErr(400, "Invalid board size"))
			return
		}
	}
	if _, err := signalDailySeed(day, size); err != nil {
		respond(w, nil, signalErr(400, "Invalid challenge"))
		return
	}
	rows, err := h.s.db.QueryContext(r.Context(), `SELECT r.user_id,r.player_name,r.actor,r.model_id,r.harness,r.agent_name,r.note,r.moves,r.elapsed_ms,r.created_at
 FROM lmm_community.signal_records r
 WHERE r.day=$1 AND r.size=$2 AND r.mode='challenge' AND r.public
 AND NOT EXISTS (SELECT 1 FROM lmm_community.signal_records b WHERE b.day=r.day AND b.size=r.size AND b.mode='challenge' AND b.public AND b.user_id=r.user_id AND b.participant_key=r.participant_key AND
 (b.moves<r.moves OR (b.moves=r.moves AND b.elapsed_ms<r.elapsed_ms) OR (b.moves=r.moves AND b.elapsed_ms=r.elapsed_ms AND b.id<r.id)))
 ORDER BY r.moves,r.elapsed_ms,r.id LIMIT 50`, day, size)
	if err != nil {
		respond(w, nil, signalErr(503, "Leaderboard unavailable"))
		return
	}
	defer rows.Close()
	entries := []signalRank{}
	for rows.Next() {
		var e signalRank
		if err = rows.Scan(&e.Player, &e.Name, &e.Actor, &e.ModelID, &e.Harness, &e.AgentName, &e.Note, &e.Moves, &e.ElapsedMs, &e.FinishedAt); err != nil {
			respond(w, nil, signalErr(503, "Leaderboard unavailable"))
			return
		}
		if e.Name == "" {
			e.Name = fmt.Sprintf("Player %d", e.Player)
		}
		entries = append(entries, e)
	}
	if rows.Err() != nil {
		respond(w, nil, signalErr(503, "Leaderboard unavailable"))
		return
	}
	respond(w, map[string]any{"day": day, "size": size, "entries": entries}, nil)
}

type signalRecord struct {
	ID           int64  `json:"id"`
	Mode         string `json:"mode"`
	Day          string `json:"day"`
	Size         int    `json:"size"`
	Seed         uint32 `json:"seed"`
	RulesVersion int    `json:"rules_version"`
	Moves        int    `json:"moves"`
	ElapsedMs    int64  `json:"elapsed_ms"`
	Hints        int    `json:"hints"`
	Actor        string `json:"actor"`
	ModelID      string `json:"model_id"`
	Harness      string `json:"harness"`
	AgentName    string `json:"agent_name"`
	Note         string `json:"note"`
	Public       bool   `json:"public"`
	CreatedAt    int64  `json:"created_at"`
}

const recordColumns = `id,mode,day,size,seed,rules_version,moves,elapsed_ms,hints,actor,model_id,harness,agent_name,note,public,created_at`

func scanRecord(row rowScanner) (signalRecord, error) {
	var rec signalRecord
	var seed int64
	err := row.Scan(&rec.ID, &rec.Mode, &rec.Day, &rec.Size, &seed, &rec.RulesVersion, &rec.Moves, &rec.ElapsedMs, &rec.Hints, &rec.Actor, &rec.ModelID, &rec.Harness, &rec.AgentName, &rec.Note, &rec.Public, &rec.CreatedAt)
	rec.Seed = uint32(seed)
	return rec, err
}

func (h *handlers) signalRecords(w http.ResponseWriter, r *http.Request) {
	actor, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	rows, err := h.s.db.QueryContext(r.Context(), `SELECT `+recordColumns+` FROM lmm_community.signal_records WHERE user_id=$1 ORDER BY id DESC LIMIT 30`, actor.GetUserId())
	if err != nil {
		respond(w, nil, signalErr(503, "Records unavailable"))
		return
	}
	defer rows.Close()
	out := []signalRecord{}
	for rows.Next() {
		rec, err := scanRecord(rows)
		if err != nil {
			respond(w, nil, signalErr(503, "Records unavailable"))
			return
		}
		out = append(out, rec)
	}
	if rows.Err() != nil {
		respond(w, nil, signalErr(503, "Records unavailable"))
		return
	}
	respond(w, out, nil)
}

func (h *handlers) signalSave(w http.ResponseWriter, r *http.Request) {
	actor, err := h.user(r)
	if err != nil {
		respond(w, nil, err)
		return
	}
	var in struct {
		ExpectedUserID int64  `json:"expected_user_id"`
		Mode           string `json:"mode"`
		Size           int    `json:"size"`
		Seed           uint32 `json:"seed"`
		RulesVersion   int    `json:"rules_version"`
		Actions        []int  `json:"actions"`
		Token          string `json:"token"`
		Publish        bool   `json:"publish"`
		Actor          string `json:"actor"`
		ModelID        string `json:"model_id"`
		Harness        string `json:"harness"`
		AgentName      string `json:"agent_name"`
		Email          string `json:"email"`
		Note           string `json:"note"`
	}
	if err = decode(w, r, 512<<10, &in, false); err != nil || in.RulesVersion != signalRulesVersion || !utf8.ValidString(in.Note) || utf8.RuneCountInString(in.Note) > 160 || len(in.Email) > 254 {
		respond(w, nil, signalErr(400, "Invalid game record"))
		return
	}
	userID := actor.GetUserId()
	if in.ExpectedUserID != 0 && in.ExpectedUserID != userID {
		respond(w, nil, signalErr(409, "Account changed before submission"))
		return
	}
	if in.Email != "" {
		if a, e := mail.ParseAddress(in.Email); e != nil || a.Address != in.Email {
			respond(w, nil, signalErr(400, "Invalid email"))
			return
		}
	}
	type row struct {
		attemptID                               *int64
		day, actor, modelID, harness, agentName string
		size, moves, hints                      int
		seed                                    uint32
		elapsedMs                               int64
		actions, digest                         string
	}
	var rec row
	switch in.Mode {
	case "challenge":
		if len(in.Token) != 43 {
			respond(w, nil, signalErr(400, "Completed challenge required"))
			return
		}
		a, e := scanAttempt(h.s.db.QueryRowContext(r.Context(), `SELECT `+attemptColumns+` FROM lmm_community.signal_attempts WHERE token_hash=$1`, signalHash(in.Token)))
		if e != nil || a.finishedAt == 0 {
			respond(w, nil, signalErr(400, "Completed challenge required"))
			return
		}
		rec = row{attemptID: &a.id, day: a.day, seed: a.seed, size: a.size, moves: a.moves, elapsedMs: a.elapsedMs, actions: a.actions,
			actor: a.actor, modelID: a.modelID, harness: a.harness, agentName: a.agentName, digest: signalHash(fmt.Sprintf("challenge:%d", a.id))}
	case "practice":
		if in.Publish || !validSignalActor(in.Actor, in.ModelID, in.Harness, in.AgentName) {
			respond(w, nil, signalErr(400, "Practice results are private"))
			return
		}
		hints, e := signalReplay(in.Seed, in.Size, in.Actions, true)
		if e != nil {
			respond(w, nil, signalErr(400, "Game replay rejected"))
			return
		}
		actions, _ := json.Marshal(in.Actions)
		rec = row{size: in.Size, seed: in.Seed, moves: len(in.Actions), hints: hints, actions: string(actions), actor: in.Actor, modelID: in.ModelID, harness: in.Harness, agentName: in.AgentName}
		rec.digest = signalHash(fmt.Sprintf("practice:%d:%d:%d:%s:%s:%s:%s:%s", userID, rec.size, rec.seed, actions, rec.actor, rec.modelID, rec.harness, rec.agentName))
	default:
		respond(w, nil, signalErr(400, "Invalid game mode"))
		return
	}
	identity, _ := json.Marshal([]string{rec.actor, rec.modelID, rec.harness, rec.agentName})
	participant := signalHash(string(identity))
	now := h.s.now()
	var saved signalRecord
	err = h.s.tx(r.Context(), func(ctx context.Context, tx *sql.Tx) error {
		if _, e := tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock(hashtextextended('lmm_community:signal:'||$1::text,0))`, userID); e != nil {
			return e
		}
		var owner int64
		e := tx.QueryRowContext(ctx, `SELECT user_id FROM lmm_community.signal_records WHERE digest=$1 FOR UPDATE`, rec.digest).Scan(&owner)
		if e == nil {
			if owner != userID {
				return signalErr(409, "This result belongs to another account")
			}
			saved, e = scanRecord(tx.QueryRowContext(ctx, `UPDATE lmm_community.signal_records SET public=public OR $2,email=$3,note=$4 WHERE digest=$1 RETURNING `+recordColumns, rec.digest, in.Publish, in.Email, in.Note))
			return e
		}
		if !errors.Is(e, sql.ErrNoRows) {
			return e
		}
		var count int
		if e = tx.QueryRowContext(ctx, `SELECT count(*) FROM lmm_community.signal_records WHERE user_id=$1 AND created_at>=$2`, userID, now.UTC().Truncate(24*time.Hour).Unix()).Scan(&count); e != nil {
			return e
		}
		if count >= 100 {
			return signalErr(429, "Daily record limit reached")
		}
		saved, e = scanRecord(tx.QueryRowContext(ctx, `INSERT INTO lmm_community.signal_records(user_id,attempt_id,mode,day,size,seed,rules_version,moves,elapsed_ms,hints,actor,model_id,harness,agent_name,participant_key,email,note,actions,digest,public,created_at)
 VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21) RETURNING `+recordColumns,
			userID, rec.attemptID, in.Mode, rec.day, rec.size, int64(rec.seed), signalRulesVersion, rec.moves, rec.elapsedMs, rec.hints, rec.actor, rec.modelID, rec.harness, rec.agentName, participant, in.Email, in.Note, rec.actions, rec.digest, in.Publish, now.Unix()))
		return e
	})
	if err != nil {
		var api *apiError
		if !errors.As(err, &api) {
			err = signalErr(503, "Could not save game record")
		}
		respond(w, nil, err)
		return
	}
	respond(w, saved, nil)
}
