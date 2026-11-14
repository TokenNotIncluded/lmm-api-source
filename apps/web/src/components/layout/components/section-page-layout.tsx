/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import {
  Children,
  isValidElement,
  useState,
  type ReactElement,
  type ReactNode,
} from 'react'

import { Main } from './main'
import { MobileScrollChrome } from './mobile-scroll-chrome'
import { PageFooterProvider } from './page-footer'

type SlotProps = { children?: ReactNode }

function SectionPageLayoutTitle(_props: SlotProps) {
  return null
}
SectionPageLayoutTitle.displayName = 'SectionPageLayout.Title'

function SectionPageLayoutActions(_props: SlotProps) {
  return null
}
SectionPageLayoutActions.displayName = 'SectionPageLayout.Actions'

function SectionPageLayoutContent(_props: SlotProps) {
  return null
}
SectionPageLayoutContent.displayName = 'SectionPageLayout.Content'

function SectionPageLayoutBreadcrumb(_props: SlotProps) {
  return null
}
SectionPageLayoutBreadcrumb.displayName = 'SectionPageLayout.Breadcrumb'

export type SectionPageLayoutProps = {
  children: ReactNode
  fixedContent?: boolean
  className?: string
}

export function SectionPageLayout(props: SectionPageLayoutProps) {
  const [footerContainer, setFooterContainer] = useState<HTMLDivElement | null>(
    null
  )

  let title: ReactNode = null
  let actions: ReactNode = null
  let content: ReactNode = null
  let breadcrumb: ReactNode = null

  Children.forEach(props.children, (node) => {
    if (!isValidElement(node)) return
    const child = node as ReactElement<SlotProps>
    if (child.type === SectionPageLayoutTitle) title = child.props.children
    else if (child.type === SectionPageLayoutActions) {
      actions = child.props.children
    } else if (child.type === SectionPageLayoutContent) {
      content = child.props.children
    } else if (child.type === SectionPageLayoutBreadcrumb) {
      breadcrumb = child.props.children
    }
  })

  return (
    <PageFooterProvider container={footerContainer}>
      <Main className={props.className}>
        <MobileScrollChrome>
          <div className='console-section-header shrink-0 px-5 pt-6 pb-6 sm:px-8 sm:pt-9 sm:pb-8'>
            {breadcrumb != null && (
              <div className='mb-3 sm:mb-4'>{breadcrumb}</div>
            )}
            <div className='console-page-heading flex flex-wrap items-center justify-between gap-x-4 gap-y-3'>
              <div className='w-full min-w-0 sm:w-auto sm:flex-1'>
                <h1 className='console-page-title break-words'>{title}</h1>
              </div>
              {actions != null && (
                <div className='console-page-actions flex w-full max-w-full min-w-0 shrink-0 flex-wrap items-center justify-start gap-2 sm:w-auto sm:justify-end'>
                  {actions}
                </div>
              )}
            </div>
          </div>
        </MobileScrollChrome>

        <div
          data-mobile-scroll-root=''
          className={
            props.fixedContent
              ? 'console-section-content min-h-0 min-w-0 flex-1 overflow-auto px-5 pt-2 pb-8 sm:overflow-hidden sm:px-8 sm:pb-12'
              : 'console-section-content min-h-0 min-w-0 flex-1 overflow-auto px-5 pt-2 pb-8 sm:px-8 sm:pb-12'
          }
        >
          {content}
        </div>

        <MobileScrollChrome>
          <div
            ref={setFooterContainer}
            className='console-section-footer bg-background shrink-0 px-5 py-4 empty:hidden sm:px-8 sm:py-5'
          />
        </MobileScrollChrome>
      </Main>
    </PageFooterProvider>
  )
}

SectionPageLayout.Title = SectionPageLayoutTitle
SectionPageLayout.Actions = SectionPageLayoutActions
SectionPageLayout.Content = SectionPageLayoutContent
SectionPageLayout.Breadcrumb = SectionPageLayoutBreadcrumb
