// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

// One observation per account. Complete covers [registration, min(now,
// registration+window)). A sparse positive proves completion, not first timing.
type funnelObservation struct {
	RegisteredAt int64
	At           *int64
	FirstIsExact bool
	Complete     bool
}

type funnelStage struct {
	ID             string   `json:"id"`
	Observed       int      `json:"observed"`
	NotObserved    int      `json:"not_observed"`
	Unknown        int      `json:"unknown"`
	MatureObserved int      `json:"mature_observed"`
	MatureKnown    int      `json:"mature_known"`
	Observing      int      `json:"observing"`
	ConversionRate *float64 `json:"conversion_rate"`
	TimingAccounts int      `json:"timing_accounts"`
	MeanSeconds    *float64 `json:"mean_seconds_from_registration"`
}

func summarizeFunnelStage(id string, now int64, days int, accounts []funnelObservation) (funnelStage, error) {
	if now <= 0 || days < 1 || days > 90 || len(accounts) > 10000 {
		return funnelStage{}, ErrInvalid
	}
	result := funnelStage{ID: id}
	var elapsed int64
	span := int64(days) * daySeconds
	for _, account := range accounts {
		if account.RegisteredAt <= 0 || account.RegisteredAt > now || account.RegisteredAt > 253402300799 || (account.At != nil && (*account.At < account.RegisteredAt || *account.At > 253402300799)) {
			return funnelStage{}, errUnavailable
		}
		end := account.RegisteredAt + span
		mature := end <= now
		if !mature {
			result.Observing++
		}
		cutoff := min(now, end)
		observed := account.At != nil && *account.At < cutoff
		// Exact first events after the cutoff prove no earlier completion.
		known := observed || account.Complete || (account.At != nil && account.FirstIsExact)
		switch {
		case observed:
			result.Observed++
			if account.FirstIsExact {
				result.TimingAccounts++
				elapsed += *account.At - account.RegisteredAt
			}
		case known:
			result.NotObserved++
		default:
			result.Unknown++
		}
		if mature && known {
			result.MatureKnown++
			if observed {
				result.MatureObserved++
			}
		}
	}
	if result.MatureKnown > 0 {
		rate := float64(result.MatureObserved) / float64(result.MatureKnown)
		result.ConversionRate = &rate
	}
	if result.TimingAccounts > 0 {
		mean := float64(elapsed) / float64(result.TimingAccounts)
		result.MeanSeconds = &mean
	}
	return result, nil
}
