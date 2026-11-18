// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package coreclient

import (
	"context"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
)

// Configuration is the effective non-secret Core projection, never the key
// or a private path. The normal dedicated payment service credential applies.
func (c *Client) PaymentEPayConfiguration(ctx context.Context) (*pb.EPayConfiguration, error) {
	return pb.NewCorePaymentsClient(c.connection).GetEPayConfiguration(c.context(ctx, ""), &pb.GetEPayConfigurationRequest{})
}
