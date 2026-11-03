// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package acquisition

import "testing"

func TestFunnelUnknownsAndSparseReceipts(t *testing.T) {
	const registered int64 = 1000
	const now = registered + 3*daySeconds
	early := registered + 20
	late := registered + 2*daySeconds
	result, err := summarizeFunnelStage("first_successful_request", now, 1, []funnelObservation{
		{RegisteredAt: registered, At: &early, FirstIsExact: true, Complete: true},
		{RegisteredAt: registered, At: &early},
		{RegisteredAt: registered, Complete: true},
		{RegisteredAt: registered},
		{RegisteredAt: registered, At: &late},
		{RegisteredAt: registered, At: &late, FirstIsExact: true},
	})
	if err != nil {
		t.Fatal(err)
	}
	if result.Observed != 2 || result.NotObserved != 2 || result.Unknown != 2 || result.MatureKnown != 4 || result.MatureObserved != 2 || result.Observing != 0 {
		t.Fatalf("counts: %#v", result)
	}
	if result.ConversionRate == nil || *result.ConversionRate != 0.5 {
		t.Fatalf("unknowns entered denominator: %#v", result)
	}
	if result.TimingAccounts != 1 || result.MeanSeconds == nil || *result.MeanSeconds != 20 {
		t.Fatalf("sparse history invented first timing: %#v", result)
	}
}

func TestFunnelHalfOpenWindowAndMaturity(t *testing.T) {
	const registered int64 = 1000
	deadline := registered + daySeconds
	before := deadline - 1
	result, err := summarizeFunnelStage("first_payment", deadline, 1, []funnelObservation{
		{RegisteredAt: registered, At: &before, FirstIsExact: true},
		{RegisteredAt: registered, At: &deadline, FirstIsExact: true},
		{RegisteredAt: registered + 1, Complete: true},
	})
	if err != nil {
		t.Fatal(err)
	}
	if result.Observed != 1 || result.NotObserved != 2 || result.MatureKnown != 2 || result.MatureObserved != 1 || result.Observing != 1 {
		t.Fatalf("boundary: %#v", result)
	}
	if result.ConversionRate == nil || *result.ConversionRate != 0.5 {
		t.Fatalf("open window entered mature denominator: %#v", result)
	}
}

func TestFunnelUnknownOrObservingHasNullRate(t *testing.T) {
	for _, accounts := range [][]funnelObservation{nil, {{RegisteredAt: 1000}}, {{RegisteredAt: 1000, Complete: true}}} {
		result, err := summarizeFunnelStage("client_configured", 1001, 1, accounts)
		if err != nil || result.ConversionRate != nil || result.MeanSeconds != nil {
			t.Fatalf("unknown became zero: %#v %v", result, err)
		}
	}
}

func TestFunnelInvalidDatesFailWithoutPartialTotals(t *testing.T) {
	before := int64(999)
	for _, account := range []funnelObservation{{RegisteredAt: 0}, {RegisteredAt: 1002}, {RegisteredAt: 1000, At: &before}} {
		_, err := summarizeFunnelStage("first_payment", 1001, 1, []funnelObservation{{RegisteredAt: 1000, Complete: true}, account})
		if err != errUnavailable {
			t.Fatalf("invalid date counted: %#v %v", account, err)
		}
	}
}
