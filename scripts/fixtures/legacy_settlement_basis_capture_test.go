// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
// Copy into model/ of the pinned original Go archive for isolated verification.
package model

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"testing"

	"github.com/LIghtJUNction/api.lmm.best/common"
	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"
)

func TestCaptureLegacySettlementBasisOffline(t *testing.T) {
	socket := os.Getenv("LMM_LEGACY_ORACLE_SOCKET")
	database := os.Getenv("LMM_LEGACY_ORACLE_DATABASE")
	output := os.Getenv("LMM_LEGACY_ORACLE_OUTPUT")
	expected, err := strconv.Atoi(os.Getenv("LMM_LEGACY_ORACLE_EXPECTED_ROWS"))
	home, e := os.UserHomeDir()
	if e != nil || err != nil || expected < 0 {
		t.Fatal("explicit local oracle scope required")
	}
	backups, e := filepath.EvalSymlinks(filepath.Join(home, ".local/state/lmm-api"))
	if e != nil {
		t.Fatal("private backup root required")
	}
	actualSocket, e := filepath.EvalSymlinks(socket)
	if e != nil || !strings.HasPrefix(actualSocket, backups+string(os.PathSeparator)) || !regexp.MustCompile(`^[A-Za-z0-9_./-]+$`).MatchString(actualSocket) {
		t.Fatal("local backup socket required")
	}
	parent, e := filepath.EvalSymlinks(filepath.Dir(output))
	if e != nil || !strings.HasPrefix(parent, backups+string(os.PathSeparator)) {
		t.Fatal("private output directory required")
	}
	if !regexp.MustCompile(`^lmm_legacy_quote_oracle_[a-z0-9_]+$`).MatchString(database) {
		t.Fatal("dedicated disposable clone required")
	}
	dsn := fmt.Sprintf("host=%s port=55436 user=postgres dbname=%s search_path=lmm_prod_20260802 sslmode=disable", actualSocket, database)
	db, err := gorm.Open(postgres.Open(dsn), &gorm.Config{Logger: logger.Default.LogMode(logger.Silent)})
	if err != nil {
		t.Fatal("isolated clone connection failed")
	}
	old := DB
	DB = db
	defer func() { DB = old }()
	common.SetDatabaseTypes(common.DatabaseTypePostgreSQL, common.DatabaseTypePostgreSQL)
	sqlDB, err := db.DB()
	if err != nil {
		t.Fatal(err)
	}
	defer sqlDB.Close()
	if err = VerifyCreditUnits(context.Background()); err != nil {
		t.Fatal("original read-only currency initialization rejected clone")
	}
	var rows []TopUp
	if err = db.Where("status='pending' OR(status='failed' AND payment_provider='waffo_pancake' AND failure_reason_code='checkout_timeout')").Order("id").Find(&rows).Error; err != nil {
		t.Fatal("source selection failed")
	}
	if len(rows) != expected {
		t.Fatal("unexpected source scope")
	}
	type result struct {
		ID             int   `json:"id"`
		UserID         int   `json:"user_id"`
		Quota          int64 `json:"quota"`
		ExpectedMicros int64 `json:"expected_micros"`
		Rejected       bool  `json:"rejected"`
	}
	out := []result{}
	rollback := errors.New("intentional rollback after observations")
	err = db.Transaction(func(tx *gorm.DB) error {
		for i := range rows {
			r := &rows[i]
			quota, e := pendingTopUpSettlementQuotaTx(tx, r)
			if e != nil && !errors.Is(e, ErrInvalidTopUpQuota) {
				return errors.New("original oracle storage or unexpected failure")
			}
			out = append(out, result{r.Id, r.UserId, quota, expectedTopUpAmountMicros(r), e != nil})
		}
		return rollback
	})
	if !errors.Is(err, rollback) {
		t.Fatal("oracle transaction failed")
	}
	body, err := json.Marshal(out)
	if err != nil {
		t.Fatal(err)
	}
	file, err := os.OpenFile(filepath.Join(parent, filepath.Base(output)), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		t.Fatal("new private output required")
	}
	if _, err = file.Write(body); err != nil {
		file.Close()
		t.Fatal(err)
	}
	if err = file.Close(); err != nil {
		t.Fatal(err)
	}
	rejected := 0
	for _, r := range out {
		if r.Rejected {
			rejected++
		}
	}
	t.Logf("original observations: %d rows, %d rejected; transaction rolled back", len(out), rejected)
}
