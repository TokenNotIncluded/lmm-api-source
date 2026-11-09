package store

import (
	"errors"
	"testing"
)

func TestSnapshotVisibilityAndLegacyTestMode(t *testing.T) {
	for _, tc := range []struct {
		visibility string
		testMode   bool
		viewer     int64
		visible    bool
	}{{"public", false, 0, true}, {"registered", false, 0, false}, {"registered", false, 2, true}, {"private", false, 2, false}, {"private", false, 1, true}, {"public", true, 0, true}, {"", true, 0, false}, {"", true, 2, false}, {"", true, 1, true}} {
		row := publicRow(t)
		row.Product.LegacyPublic = publicSnapshot(t, map[string]any{"visibility": tc.visibility, "test_mode": tc.testMode})
		view, err := publicViewFor(row, tc.viewer)
		if tc.visible {
			must(t, err)
			if view["review_note"] != "" || view["reviewed_by"] != 0 {
				t.Fatal("private moderation details leaked")
			}
		} else if !errors.Is(err, ErrNotFound) {
			t.Fatalf("%+v: %v", tc, err)
		}
	}
}
