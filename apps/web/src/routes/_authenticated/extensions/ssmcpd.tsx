import { createFileRoute } from '@tanstack/react-router'

import { SSMCPDPage } from '@/features/plugins/ssmcpd'

export const Route = createFileRoute('/_authenticated/extensions/ssmcpd')({
  component: SSMCPDPage,
})
