package webcompat

import (
	"net/http/httptest"
	"strings"
	"testing"
)

func TestStoreInventoryRoutesUseAuthenticatedLegacyPort(t *testing.T) {
	for _, route := range []struct{ method, path, action, child, stock, body string }{
		{"GET", "/api/store/products/product-1/inventory?offset=2&limit=20", "stock_list", "", "", ""},
		{"POST", "/api/store/products/product-1/inventory", "stock_add", "", "", `{"items":["synthetic"]}`},
		{"DELETE", "/api/store/products/product-1/inventory/item-1", "stock_remove", "", "item-1", ""},
		{"GET", "/api/store/products/product-1/variants/variant-1/inventory", "stock_list", "variant-1", "", ""},
		{"POST", "/api/store/products/product-1/variants/variant-1/inventory", "stock_add", "variant-1", "", `{"items":["synthetic"]}`},
		{"DELETE", "/api/store/products/product-1/variants/variant-1/inventory/item-1", "stock_remove", "variant-1", "item-1", ""},
	} {
		core, service := &authCore{}, newStoreLegacyTransportFake()
		req := httptest.NewRequest(route.method, route.path, strings.NewReader(route.body))
		req.Header.Set("Authorization", "Bearer "+webToken)
		rec := httptest.NewRecorder()
		storeTransportMux(t, core, service).ServeHTTP(rec, req)
		if rec.Code != 200 || len(service.requests) != 1 {
			t.Fatalf("%s %s status=%d body=%s", route.method, route.path, rec.Code, rec.Body.String())
		}
		got := service.requests[0]
		if got.Action != route.action || got.ID != "product-1" || got.ChildID != route.child || got.StockID != route.stock {
			t.Fatalf("wrong dispatch %+v", got)
		}
		rec = httptest.NewRecorder()
		storeTransportMux(t, &authCore{}, service).ServeHTTP(rec, httptest.NewRequest(route.method, route.path, strings.NewReader(route.body)))
		if rec.Code != 401 || len(service.requests) != 1 {
			t.Fatal("anonymous inventory reached domain")
		}
	}
}
