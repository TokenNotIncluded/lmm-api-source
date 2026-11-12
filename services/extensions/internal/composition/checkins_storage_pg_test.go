package composition

import (
	"context"
	"os"
	"testing"
)

func TestCheckinsRealRuntimeStorageAuthority(t *testing.T) {
	path := os.Getenv("LMM_CHECKINS_TEST_RUNTIME_URL_FILE")
	if path == "" {
		t.Skip("requires explicit synthetic checkins storage")
	}
	db, e := openStorage(context.Background(), StorageConfig{Component: "checkins", URLFile: path, Database: "lmm_checkins_request_test"})
	if e != nil {
		t.Fatal(e)
	}
	defer db.Close()
	var marker string
	if e = db.QueryRow("SELECT current_setting('lmm.synthetic',true)").Scan(&marker); e != nil || marker != "checkins-extension-test" {
		t.Fatal("not synthetic test storage")
	}
}
