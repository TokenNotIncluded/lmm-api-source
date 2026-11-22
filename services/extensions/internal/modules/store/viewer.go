package store

import "context"

// ProductForViewer shares the normal catalogue visibility and pricing projection.
// The viewer is revalidated through Core; a product ID or platform role does not
// authorize another merchant's private preview. It never invokes checkout.
func (s *Service) ProductForViewer(ctx context.Context, token string, viewer int64, id string) (map[string]any, error) {
	if s == nil || viewer <= 0 || !Text(id, 128) {
		return nil, ErrInvalid
	}
	actor, err := s.check(ctx, token, Account{Kind: "personal", ID: viewer}, Read)
	if err != nil {
		return nil, err
	}
	if actor != viewer {
		return nil, ErrForbidden
	}
	row, err := s.LocateProduct(ctx, id)
	if err != nil {
		return nil, err
	}
	return s.publicLegacyViewFor(row, true, viewer)
}
