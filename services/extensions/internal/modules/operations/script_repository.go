// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package operations

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"net/http"
	"regexp"
	"strings"
	"time"
)

// ScriptRepository uses an owner-installed singleton. It does no DDL and must
// only receive the dedicated operations runtime pool after installation checks.
// HTTP callers must freshly authorize administrators before invoking Settings,
// Save or Pull, just as for RSS. List and Raw are public content projections.
type ScriptRepository struct {
	db     *sql.DB
	client *http.Client
}

func NewScriptRepository(db *sql.DB) *ScriptRepository {
	return &ScriptRepository{db: db, client: newRSS(nil).client}
}

type ScriptRepositorySettings struct {
	Revision         int64  `json:"revision"`
	URL              string `json:"repository_url"`
	Branch           string `json:"branch"`
	GitHubKeySet     bool   `json:"github_key_set"`
	LastPulledAt     string `json:"last_pulled_at,omitempty"`
	SnapshotURL      string `json:"snapshot_repository_url,omitempty"`
	SnapshotBranch   string `json:"snapshot_branch,omitempty"`
	SnapshotPulledAt string `json:"snapshot_pulled_at,omitempty"`
}

var ErrNoScriptSnapshot = errors.New("script repository has no imported snapshot")

var ErrRepositoryCredentials = errors.New("private script repository credentials unsupported")

var repositoryPath = regexp.MustCompile(`^[A-Za-z0-9_-]+/[A-Za-z0-9_.-]+$`)
var repositoryBranch = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$`)

func validateScriptRepository(settings ScriptRepositorySettings) (ScriptRepositorySettings, error) {
	if strings.Contains(settings.URL, "#") {
		return ScriptRepositorySettings{}, ErrInvalid
	}
	u, err := rssURL(settings.URL)
	if err != nil || u.Scheme != "https" || u.Host != "github.com" || u.RawQuery != "" || u.Fragment != "" || u.RawPath != "" {
		return ScriptRepositorySettings{}, ErrInvalid
	}
	name := strings.TrimSuffix(strings.TrimPrefix(u.Path, "/"), ".git")
	if !repositoryPath.MatchString(name) || strings.HasSuffix(name, "/.") || strings.HasSuffix(name, "/..") || len(name) > 200 || !repositoryBranch.MatchString(settings.Branch) || strings.Contains(settings.Branch, "..") || settings.Revision < 0 || settings.Revision >= MaxID || settings.GitHubKeySet {
		return ScriptRepositorySettings{}, ErrInvalid
	}
	for _, part := range strings.Split(settings.Branch, "/") {
		if part == "" || strings.HasPrefix(part, ".") || strings.HasSuffix(part, ".") || strings.HasSuffix(part, ".lock") {
			return ScriptRepositorySettings{}, ErrInvalid
		}
	}
	settings.URL = "https://github.com/" + name
	return settings, nil
}

func (s *ScriptRepository) Settings(ctx context.Context) (ScriptRepositorySettings, error) {
	if s == nil || s.db == nil {
		return ScriptRepositorySettings{}, ErrUnavailable
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var settings ScriptRepositorySettings
	var pulled sql.NullTime
	err := s.db.QueryRowContext(ctx, `SELECT revision,repository_url,branch,snapshot_repository_url,snapshot_branch,snapshot_pulled_at FROM lmm_operations.script_repository WHERE singleton`).Scan(&settings.Revision, &settings.URL, &settings.Branch, &settings.SnapshotURL, &settings.SnapshotBranch, &pulled)
	if err != nil {
		return ScriptRepositorySettings{}, ErrUnavailable
	}
	if pulled.Valid {
		settings.SnapshotPulledAt = pulled.Time.UTC().Format(time.RFC3339Nano)
		if settings.URL == settings.SnapshotURL && settings.Branch == settings.SnapshotBranch {
			settings.LastPulledAt = settings.SnapshotPulledAt
		}
	}
	return settings, nil
}

// Save requires the revision read by the administrator. Changing the source
// retains the last complete snapshot until a new pull succeeds. No credentials
// are stored by this public-GitHub-only slice.
func (s *ScriptRepository) Save(ctx context.Context, actor int64, settings ScriptRepositorySettings) (ScriptRepositorySettings, error) {
	if s == nil || s.db == nil {
		return ScriptRepositorySettings{}, ErrUnavailable
	}
	settings, err := validateScriptRepository(settings)
	if err != nil || !validID(actor) {
		return ScriptRepositorySettings{}, ErrInvalid
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var pulled sql.NullTime
	err = s.db.QueryRowContext(ctx, `UPDATE lmm_operations.script_repository SET repository_url=$1,branch=$2,revision=revision+1,updated_by=$3 WHERE singleton AND revision=$4 RETURNING revision,snapshot_repository_url,snapshot_branch,snapshot_pulled_at`, settings.URL, settings.Branch, actor, settings.Revision).Scan(&settings.Revision, &settings.SnapshotURL, &settings.SnapshotBranch, &pulled)
	if errors.Is(err, sql.ErrNoRows) {
		return ScriptRepositorySettings{}, ErrConflict
	}
	if err != nil {
		return ScriptRepositorySettings{}, ErrUnavailable
	}
	settings.LastPulledAt = ""
	if pulled.Valid {
		settings.SnapshotPulledAt = pulled.Time.UTC().Format(time.RFC3339Nano)
		if settings.URL == settings.SnapshotURL && settings.Branch == settings.SnapshotBranch {
			settings.LastPulledAt = settings.SnapshotPulledAt
		}
	}
	return settings, nil
}

func (s *ScriptRepository) Pull(ctx context.Context, actor int64) (ScriptRepositorySettings, error) {
	if !validID(actor) {
		return ScriptRepositorySettings{}, ErrInvalid
	}
	settings, err := s.Settings(ctx)
	if err != nil {
		return ScriptRepositorySettings{}, err
	}
	settings, err = validateScriptRepository(settings)
	if err != nil {
		return ScriptRepositorySettings{}, err
	}
	// No transaction or DB lock is held while fetching. The revision also
	// advances on pull, preventing a slower concurrent fetch from replacing a
	// newer completed pull or importing a superseded repository configuration.
	snapshot, err := fetchScriptRepository(ctx, s.client, settings)
	if err != nil {
		return ScriptRepositorySettings{}, err
	}
	files := make(map[string][]byte, len(snapshot.entries))
	for name, entry := range snapshot.entries {
		files[name] = entry.data
	}
	raw, err := json.Marshal(files)
	if err != nil {
		return ScriptRepositorySettings{}, ErrUnavailable
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var pulled time.Time
	err = s.db.QueryRowContext(ctx, `UPDATE lmm_operations.script_repository SET scripts=$1,snapshot_repository_url=repository_url,snapshot_branch=branch,snapshot_pulled_at=clock_timestamp(),updated_by=$2,revision=revision+1 WHERE singleton AND revision=$3 RETURNING revision,snapshot_repository_url,snapshot_branch,snapshot_pulled_at`, string(raw), actor, settings.Revision).Scan(&settings.Revision, &settings.SnapshotURL, &settings.SnapshotBranch, &pulled)
	if errors.Is(err, sql.ErrNoRows) {
		return ScriptRepositorySettings{}, ErrConflict
	}
	if err != nil {
		return ScriptRepositorySettings{}, ErrUnavailable
	}
	settings.SnapshotPulledAt = pulled.UTC().Format(time.RFC3339Nano)
	settings.LastPulledAt = settings.SnapshotPulledAt
	return settings, nil
}

func (s *ScriptRepository) snapshot(ctx context.Context) (*Scripts, error) {
	if s == nil || s.db == nil {
		return nil, ErrUnavailable
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var raw []byte
	var pulled sql.NullTime
	if err := s.db.QueryRowContext(ctx, `SELECT scripts,snapshot_pulled_at FROM lmm_operations.script_repository WHERE singleton`).Scan(&raw, &pulled); err != nil {
		return nil, ErrUnavailable
	}
	if raw == nil {
		return nil, ErrNoScriptSnapshot
	}
	var files map[string][]byte
	if json.Unmarshal(raw, &files) != nil {
		return nil, ErrUnavailable
	}
	return repositoryScripts(files, pulled.Time)
}

func (s *ScriptRepository) List(ctx context.Context) ([]ScriptMeta, error) {
	snapshot, err := s.snapshot(ctx)
	if err != nil {
		return nil, err
	}
	return snapshot.List()
}

func (s *ScriptRepository) Raw(ctx context.Context, name string) ([]byte, error) {
	if !ValidScriptName(name) {
		return nil, ErrInvalid
	}
	snapshot, err := s.snapshot(ctx)
	if err != nil {
		return nil, err
	}
	return snapshot.Raw(name)
}
