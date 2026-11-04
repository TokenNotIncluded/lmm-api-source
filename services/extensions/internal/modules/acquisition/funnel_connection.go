package acquisition

import "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/community"

// Unknown includes missing historical coverage, not just a known absence of
// connection. An exclusive method requires both sides to be known.
func connectionObservations(u funnelAccount, fact community.ConnectionMilestone, now int64, days int) (key, oauth funnelObservation, method string, err error) {
	key = funnelObservation{RegisteredAt: u.Registered, At: fact.APIKeyCreatedAt, Complete: fact.HistoryComplete, FirstIsExact: fact.HistoryComplete}
	oauth = funnelObservation{RegisteredAt: u.Registered, At: fact.OAuthAuthorizedAt, Complete: fact.HistoryComplete, FirstIsExact: fact.HistoryComplete}
	cutoff := min(now, u.Registered+int64(days)*daySeconds)
	within := func(at *int64) bool { return at != nil && *at >= u.Registered && *at < cutoff }
	for _, at := range []*int64{key.At, oauth.At} {
		if at != nil && (*at < u.Registered || *at > now) {
			return key, oauth, "", errUnavailable
		}
	}
	k, o := within(key.At), within(oauth.At)
	method = "unknown"
	switch {
	case k && o:
		method = "both"
	case fact.HistoryComplete && k:
		method = "api_key"
	case fact.HistoryComplete && o:
		method = "oauth"
	}
	return
}
