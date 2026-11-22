package coreclient

import (
	"context"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/invitationfacts"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func (c *AssistantClient) InvitationFactsReady(ctx context.Context) error {
	return c.assistantFeatures(ctx, invitationfacts.Feature)
}
func (c *AssistantClient) GetAssistantInvitationFacts(ctx context.Context, user string) (invitationfacts.Facts, error) {
	raw, e := c.GetAssistantBusinessFacts(ctx, user, "invitation_rewards")
	if e != nil {
		return invitationfacts.Facts{}, e
	}
	var out invitationfacts.Facts
	if !decodeBusinessFacts(raw, &out) || !out.Valid(out.ActorUserID) {
		return out, status.Error(codes.DataLoss, "invalid invitation facts response")
	}
	return out, nil
}
