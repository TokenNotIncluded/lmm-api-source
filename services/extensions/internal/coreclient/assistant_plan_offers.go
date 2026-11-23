package coreclient

import (
	"context"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/planoffers"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func (c *AssistantClient) PlanOffersReady(ctx context.Context) error {
	return c.assistantFeatures(ctx, planoffers.Feature)
}
func (c *AssistantClient) GetAssistantPlanOffers(ctx context.Context, user string) (planoffers.Facts, error) {
	raw, err := c.GetAssistantBusinessFacts(ctx, user, "plan_offers")
	if err != nil {
		return planoffers.Facts{}, err
	}
	var out planoffers.Facts
	if !decodeBusinessFacts(raw, &out) || !out.Valid(out.ActorUserID) {
		return planoffers.Facts{}, status.Error(codes.DataLoss, "invalid plan offers response")
	}
	return out, nil
}
