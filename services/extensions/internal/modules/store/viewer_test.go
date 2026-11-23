package store

import (
	"context"
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"testing"
)

func TestAssistantProductViewerUsesRealVisibilityWithoutWrites(t *testing.T) {
	for _, tc := range []struct {
		visibility, token string
		viewer            int64
		visible           bool
	}{
		{"public", buyerToken, 2, true}, {"registered", buyerToken, 2, true},
		{"private", sellerToken, 1, true}, {"private", buyerToken, 2, false},
	} {
		t.Run(tc.visibility+tc.token[:1], func(t *testing.T) {
			row := publicRow(t)
			row.Product.LegacyPublic = publicSnapshot(t, map[string]any{"visibility": tc.visibility})
			repo := &webCatalogueRepo{memoryRepo: newRepo(), rows: []LocatedProduct{row}}
			funds := newFunds()
			a := &authority{}
			s, err := New(repo, a, funds)
			must(t, err)
			before, err := json.Marshal(repo.rows)
			must(t, err)
			view, err := s.ProductForViewer(context.Background(), tc.token, tc.viewer, row.Product.ID)
			if !tc.visible {
				if !errors.Is(err, ErrNotFound) || view != nil {
					t.Fatal(view, err)
				}
			} else {
				must(t, err)
				encoded, err := json.Marshal(view)
				must(t, err)
				if view["id"] != row.Product.ID || view["title"] != row.Product.Title || strings.Contains(string(encoded), "PRIVATE_REVIEW_NOTE") {
					t.Fatal(string(encoded))
				}
			}
			after, err := json.Marshal(repo.rows)
			must(t, err)
			if string(before) != string(after) || len(repo.data) != 0 || len(funds.commands) != 0 || len(funds.effects) != 0 {
				t.Fatal("read changed records or invoked funds")
			}
		})
	}
}

func TestAssistantProductViewerCannotChooseAnotherOwnerOrUseRevokedSession(t *testing.T) {
	row := publicRow(t)
	row.Product.LegacyPublic = publicSnapshot(t, map[string]any{"visibility": "private"})
	repo := &webCatalogueRepo{memoryRepo: newRepo(), rows: []LocatedProduct{row}}
	a := &authority{}
	s, err := New(repo, a, nil)
	must(t, err)
	for _, tc := range []struct {
		token  string
		viewer int64
		want   error
	}{
		{buyerToken, 1, ErrForbidden}, {sellerToken, 2, ErrForbidden}, {"host-token", 1, ErrUnauthorized}, {sellerToken, 0, ErrInvalid},
	} {
		if view, err := s.ProductForViewer(context.Background(), tc.token, tc.viewer, row.Product.ID); !errors.Is(err, tc.want) || view != nil {
			t.Fatal(view, err, tc.want)
		}
	}
	a.revoked.Store(true)
	if _, err = s.ProductForViewer(context.Background(), sellerToken, 1, row.Product.ID); !errors.Is(err, ErrForbidden) {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(repo.data, map[string]map[string][]byte{}) {
		t.Fatal("read created state")
	}
}

type mismatchedViewerAuthority struct{}

func (mismatchedViewerAuthority) Check(context.Context, string, Account, Permission) (int64, error) {
	return 99, nil
}
func TestAssistantProductViewerRejectsMismatchedReturnedPrincipal(t *testing.T) {
	repo := &webCatalogueRepo{memoryRepo: newRepo(), rows: []LocatedProduct{publicRow(t)}}
	s, err := New(repo, mismatchedViewerAuthority{}, nil)
	must(t, err)
	if _, err = s.ProductForViewer(context.Background(), sellerToken, 1, "product-1"); !errors.Is(err, ErrForbidden) {
		t.Fatal(err)
	}
}
