// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"os"
	"testing"
)

// Rust owns the actual payment authority, signed synthetic events and UDS router.
func TestCoreCashPaymentsLive(t *testing.T) {
	socket := os.Getenv("LMM_TEST_CORE_CASH_SOCKET")
	if socket == "" {
		t.Skip("requires Rust-owned credited payment fixture")
	}
	client, err := NewCoreRead(socket, []byte("synthetic-community-purpose-token-20261104"))
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	ctx := context.Background()
	if err = client.ReadyNativeCashPayments(ctx); err != nil {
		t.Fatal(err)
	}
	result, err := client.NativeFirstCashPayments(ctx, []int64{101, 102, 999})
	if err != nil || !result.NativeOnly || len(result.Users) != 2 {
		t.Fatalf("cash payment response: %#v %v", result, err)
	}
	if result.Users[0].UserID != 101 || result.Users[0].FirstPaidAt == nil || *result.Users[0].FirstPaidAt <= 0 {
		t.Fatalf("verified native cash payment missing: %#v", result.Users[0])
	}
	if result.Users[1].UserID != 102 || result.Users[1].FirstPaidAt != nil {
		t.Fatalf("test-mode credit counted as cash: %#v", result.Users[1])
	}
	if err = client.ReadyFirstCashPayments(ctx); err != nil {
		t.Fatal(err)
	}
	if err = client.ReadyRepeatCashPayments(ctx); err != nil {
		t.Fatal(err)
	}
	combined, err := client.FirstCashPayments(ctx, []int64{101, 102, 999})
	if err != nil || len(combined.Users) != 2 || !combined.Users[0].HistoryComplete || combined.Users[0].FirstPaidAt == nil || *combined.Users[0].FirstPaidAt != *result.Users[0].FirstPaidAt || !combined.Users[1].HistoryComplete || combined.Users[1].FirstPaidAt != nil {
		t.Fatalf("combined payment response: %#v %v", combined, err)
	}
	if combined.Users[0].SecondPaidAt == nil || *combined.Users[0].SecondPaidAt < *combined.Users[0].FirstPaidAt || combined.Users[1].SecondPaidAt != nil {
		t.Fatalf("repeat cash payment response: %#v", combined)
	}
}
