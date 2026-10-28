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
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { getFeedbackIssueUrl } from './feedback-issue-links'

function templateFrom(url: string): string | null {
  return new URL(url).searchParams.get('template')
}

describe('feedback issue links', () => {
  test('uses the Chinese forms for simplified and traditional Chinese', () => {
    assert.equal(
      templateFrom(getFeedbackIssueUrl('frontend', 'zh-CN')),
      'frontend_improvement.yml'
    )
    assert.equal(
      templateFrom(getFeedbackIssueUrl('feature', 'zhTW')),
      'feature_request.yml'
    )
    assert.equal(
      templateFrom(getFeedbackIssueUrl('bug', 'zh')),
      'bug_report.yml'
    )
  })

  test('uses the English forms for every other supported locale', () => {
    for (const language of ['en', 'fr', 'ja', 'ru', 'vi']) {
      assert.equal(
        templateFrom(getFeedbackIssueUrl('frontend', language)),
        'frontend_improvement_en.yml'
      )
      assert.equal(
        templateFrom(getFeedbackIssueUrl('feature', language)),
        'feature_request_en.yml'
      )
      assert.equal(
        templateFrom(getFeedbackIssueUrl('bug', language)),
        'bug_report_en.yml'
      )
    }
  })
})
