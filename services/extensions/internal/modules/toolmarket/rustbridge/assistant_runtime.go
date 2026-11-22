// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package rustbridge

import (
	"context"
	market "github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/toolmarket"
)

var _ market.CoreAssistantMarket = (*Adapter)(nil)

func (a *Adapter) ReserveAssistantCredits(ctx context.Context, credential, operation string, in market.LegacyCreditReserve) (market.LegacyCreditCall, error) {
	var out market.LegacyCreditCall
	request := struct {
		Request                  market.LegacyCreditReserve `json:"request"`
		AssistantToolOperationID string                     `json:"assistant_tool_operation_id"`
	}{Request: in, AssistantToolOperationID: operation}
	err := a.requestVersion(ctx, 2, "assistant/reserve", credential, request, &out)
	return out, err
}
func (a *Adapter) ReadAssistantMarketCatalogue(ctx context.Context, credential string, in market.AssistantMarketCatalogueRequest) (market.AssistantMarketCatalogueReceipt, error) {
	var out market.AssistantMarketCatalogueReceipt
	err := a.requestVersion(ctx, 2, "assistant/catalogue", credential, in, &out)
	return out, err
}
