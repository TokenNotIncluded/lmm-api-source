package payments

import (
	"encoding/json"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"net/http/httptest"
	"testing"
)

func TestSubscriptionJSONPreservesIntegerPrecision(t *testing.T) {
	w := httptest.NewRecorder()
	subscriptionJSON(w, &pb.EPaySubscriptionOrder{AccountId: 9007199254740993, PlanId: 9007199254740995, PlanJson: []byte(`{"id":1}`)})
	var got map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got["account_id"] != "9007199254740993" || got["plan_id"] != "9007199254740995" || got["plan_json"] != "eyJpZCI6MX0=" {
		t.Fatalf("unsafe DTO: %s", w.Body.String())
	}
	if _, present := got["confirmed_at_unix"]; present {
		t.Fatal("absent optional timestamp was invented")
	}
}
