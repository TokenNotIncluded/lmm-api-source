package coreclient

import (
	"context"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/adminskillfacts"
	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func (c *AssistantClient) AdminSkillFactsReady(ctx context.Context) error {
	return c.assistantFeatures(ctx, adminskillfacts.Feature)
}

// GetAssistantAdminSkillAuthorization always asks Core using the original
// personal Session. No host-supplied role or policy becomes authority.
func (c *AssistantClient) GetAssistantAdminSkillAuthorization(ctx context.Context, credential string, target int64) (adminskillfacts.Authorization, error) {
	var out adminskillfacts.Authorization
	if target <= 0 || target > adminskillfacts.MaxSafeInteger {
		return out, status.Error(codes.InvalidArgument, "invalid administrator skill target")
	}
	if !validUserCredential(credential) {
		return out, status.Error(codes.Unauthenticated, "invalid user credential")
	}
	if err := c.AdminSkillFactsReady(ctx); err != nil {
		return out, err
	}
	actor, err := c.control.Authorize(ctx, credential)
	if err != nil {
		return out, err
	}
	if !assistantMessage(actor) || actor.GetUserId() <= 0 ||
		actor.GetCredentialKind() != pb.CredentialKind_CREDENTIAL_KIND_SESSION ||
		actor.GetOwner().GetKind() != pb.AccountKind_ACCOUNT_KIND_PERSONAL ||
		actor.GetOwner().GetId() != actor.GetUserId() {
		return out, status.Error(codes.PermissionDenied, "personal Session required")
	}
	response, err := pb.NewCoreAssistantModelClient(c.rpc.connection).GetAssistantBusinessFacts(
		c.rpc.context(ctx, credential),
		&pb.GetAssistantBusinessFactsRequest{
			Kind:         pb.AssistantBusinessFactKind_ASSISTANT_BUSINESS_FACT_KIND_ADMIN_USER_SKILLS_AUTHORIZATION,
			TargetUserId: target,
		},
	)
	if err != nil {
		return out, err
	}
	if !assistantMessage(response) || len(response.GetFactsJson()) > 4096 ||
		!decodeBusinessFacts(response.GetFactsJson(), &out) || !out.Valid(actor.GetUserId(), target) {
		return adminskillfacts.Authorization{}, status.Error(codes.DataLoss, "invalid administrator skill authorization")
	}
	return out, nil
}
