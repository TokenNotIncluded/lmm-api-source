package freshinstall

import (
	"strings"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/store"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/support"
)

func compiledCommerce(component string) (plan, error) {
	p := plan{prefix: "lmm_" + component, namespaces: []string{"lmm_" + component}}
	switch component {
	case "store":
		// The fresh schema already contains the changes from migrations 002–006.
		// Reapplying those upgrade scripts would duplicate constraints and tables.
		p.schema = store.Schema
		p.grants = []string{
			"GRANT SELECT,INSERT,UPDATE ON lmm_store.shops,lmm_store.products,lmm_store.variants,lmm_store.orders,lmm_store.commands,lmm_store.stock_movements,lmm_store.money_operations,lmm_store.stock_items,lmm_store.product_likes,lmm_store.placements,lmm_store.extore_flows,lmm_store.extore_connections,lmm_store.extore_products TO ",
			"GRANT SELECT,INSERT,UPDATE,DELETE ON lmm_store.traffic_days,lmm_store.traffic_receipts,lmm_store.cart_items,lmm_store.favorites TO ",
		}
	case "support":
		p.schema = support.Schema
		p.grants = []string{
			"GRANT SELECT,INSERT,UPDATE ON lmm_support.customers,lmm_support.conversations,lmm_support.messages,lmm_support.commands TO ",
		}
	default:
		return plan{}, ErrRejected
	}
	// Keep the compiled schema inside Install's one serializable transaction.
	if strings.Count(p.schema, "BEGIN;") != 1 || strings.Count(p.schema, "COMMIT;") != 1 {
		return plan{}, ErrRejected
	}
	p.schema = strings.Replace(p.schema, "BEGIN;", "", 1)
	p.schema = strings.Replace(p.schema, "COMMIT;", "", 1)
	return p, nil
}
