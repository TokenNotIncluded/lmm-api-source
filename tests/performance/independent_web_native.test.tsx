/** Independent captured-Native -> current Web contract. No live server/browser. */
import { mock, test } from 'bun:test'
import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createInstance } from 'i18next'
import { I18nextProvider, initReactI18next } from 'react-i18next'
import { Window } from 'happy-dom'

const root = process.env.INDEPENDENT_WEB_EVIDENCE!
const read = (name: string) => JSON.parse(readFileSync(join(root, 'payloads', name + '.json'), 'utf8'))
const full = read('summary-missing')
const detail = read('detail-hour')
const empty = read('summary-empty')
const emptyDetail = read('detail-empty')
const samples = read('rows-final')
let response = full
const calls: unknown[] = []
mock.module('@/lib/http-client', () => ({ api: { get: async (path: string, config: unknown) => {
  assert.ok(path === '/api/perf-metrics' || path === '/api/perf-metrics/summary', 'Unexpected request '+path); calls.push({ path, config }); return { data: structuredClone(response) }
} } }))
// Only canvas rendering is replaced; real detail component/table/format/series run.
mock.module('@/features/pricing/components/model-details-charts', () => ({
  LatencyTrendChart: (props: any) => <pre data-contract-chart='latency'>{JSON.stringify(props.series)}</pre>,
  SuccessRateTrendChart: (props: any) => <pre data-contract-chart='success'>{JSON.stringify(props.series)}</pre>,
}))
const api = await import('@/features/performance-metrics/api')
const format = await import('@/features/performance-metrics/lib/format')
const series = await import('@/features/pricing/lib/performance-series')
const status = await import('@/features/status-detection/lib/aggregate')
const { buildPerfMap } = await import('@/features/pricing/hooks/use-perf-map')
const { PerformanceOverview } = await import('@/features/dashboard/components/models/performance-overview')
const { PerformanceHealthPanel } = await import('@/features/dashboard/components/overview/performance-health-panel')
const { ModelPerfBadge } = await import('@/features/pricing/components/model-perf-badge')
const { ModelDetailsPerformance } = await import('@/features/pricing/components/model-details-performance')
const i18n = createInstance()
await i18n.use(initReactI18next).init({ lng: 'en', fallbackLng: 'en', resources: { en: { translation: {} } }, interpolation: { escapeValue: false }, initImmediate: false })
const results: Record<string, string> = {}
function persist() { writeFileSync(join(root, 'results.json'), JSON.stringify(results, null, 2)) }
async function check(name: string, fn: () => unknown) {
  results[name] = 'RUNNING'; persist()
  try { await fn(); results[name] = 'PASS'; persist() }
  catch (error) { results[name] = 'FAIL'; persist(); writeFileSync(join(root, 'failure.txt'), String((error as Error).stack)); throw error }
}
function near(actual: number | null, expected: number | null) {
  if (expected === null) assert.equal(actual, null)
  else assert.ok(actual !== null && Math.abs(actual - expected) < 1e-8, `${actual} != ${expected}`)
}
// Oracle uses independent raw SQL observation facts, not any Web aggregation helper.
function facts(payload: any, model?: string, group?: string) {
  return samples.filter((r: any) => r.detail && (!model || r.model === model) && (!group || r.group === group) && r.detail.finished_at >= payload.coverage.window_start && r.detail.finished_at < payload.coverage.window_end_exclusive).map((r: any) => r.detail)
}
function expected(rows: any[]) {
  const first = rows.filter(r => r.first_output_us !== null)
  const good = rows.filter(r => r.success && r.output_tokens !== null && r.elapsed_us > 0)
  const sum = (rs: any[], key: string) => rs.reduce((s, r) => s + r[key], 0)
  return {
    avg_latency_ms: rows.length ? sum(rows, 'elapsed_us') / rows.length / 1000 : null,
    avg_ttft_ms: first.length ? sum(first, 'first_output_us') / first.length / 1000 : null,
    avg_tps: good.length ? sum(good, 'output_tokens') / sum(good, 'elapsed_us') * 1e6 : null,
    success_rate: rows.length ? rows.filter(r => r.success).length / rows.length * 100 : null,
  }
}
function render(name: string, component: React.ReactNode, payload: any) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  client.setQueryData(['perf-metrics-summary', 24], payload)
  client.setQueryData(['perf-metrics', payload.data.model_name ?? 'local-model'], payload)
  const html = renderToStaticMarkup(<I18nextProvider i18n={i18n}><QueryClientProvider client={client}>{component}</QueryClientProvider></I18nextProvider>)
  writeFileSync(join(root, name + '.html'), html)
  const window = new Window(); window.document.body.innerHTML = html
  const text = window.document.body.textContent ?? ''
  client.clear()
  return { window, document: window.document, text }
}
function assertSummary(dom: ReturnType<typeof render>) {
  // Computed from the captured raw observations: 6 requests, 5 successes.
  const e = expected(facts(full))
  assert.ok(dom.text.includes(e.success_rate!.toFixed(2) + '%'), dom.text)
  assert.ok(dom.text.includes(Math.round(e.avg_latency_ms!) + 'ms'), dom.text)
  assert.ok(dom.text.includes(e.avg_tps!.toFixed(1) + ' t/s'), dom.text)
  assert.ok(dom.text.includes('6 sampled requests; 1 missing samples'), dom.text)
  assert.ok(dom.text.includes('historical imports excluded'), dom.text)
  assert.ok(dom.document.querySelector('[role="status"]'))
  assert.ok(dom.document.querySelector('.text-warning'))
  assert.ok(!dom.text.includes('NaN') && !dom.text.includes('Infinity'))
  dom.window.close()
}

test('independent captured Native data contract; stop at first failed group', async () => {
  await check('actual-payload-schema-and-api-adapter', async () => {
    response = full; assert.deepEqual(await api.getPerfMetricsSummary(24), full)
    response = detail; assert.deepEqual(await api.getPerfMetrics('local-model', 24, 3600), detail)
    assert.deepEqual(calls, [{ path: '/api/perf-metrics/summary', config: { params: { hours: 24 } } }, { path: '/api/perf-metrics', config: { params: { model: 'local-model', hours: 24, bucket_seconds: 3600 } } }])
    const statKeys = ['request_count','success_count','latency_sum_us','ttft_count','ttft_sum_us','output_tokens','throughput_time_us','throughput_count','avg_latency_ms','avg_ttft_ms','avg_tps','success_rate'].sort()
    assert.deepEqual(Object.keys(full.data.aggregate).sort(), statKeys)
    assert.equal(detail.data.series_schema, 'core.performance.v1')
    for (const group of detail.data.groups) for (const point of group.series) assert.deepEqual(Object.keys(point).sort(), [...statKeys,'ts'].sort())
    assert.equal(full.availability, 'partial');assert.equal(full.coverage.missing_samples, 1)
  })
  await check('actual-unequal-model-and-group-weights', () => {
    const global = expected(facts(full));const local = expected(facts(detail,'local-model'))
    assert.deepEqual(full.data.models.map((m:any)=>m.request_count),[5,1])
    assert.deepEqual(detail.data.groups.map((g:any)=>g.request_count),[3,1])
    for (const metric of ['avg_latency_ms','avg_ttft_ms','avg_tps','success_rate'] as const) {
      near(format.aggregatePerformanceMetric(full.data.models,metric),global[metric])
      near(format.aggregatePerformanceMetric(detail.data.groups,metric),local[metric])
    }
    assert.notEqual(global.avg_latency_ms, full.data.models.reduce((s:number,m:any)=>s+m.avg_latency_ms,0)/2)
    assert.notEqual(local.avg_tps, detail.data.groups.reduce((s:number,g:any)=>s+g.avg_tps,0)/2)
  })
  await check('actual-detail-series-UTC-and-null', () => {
    for (const name of ['detail-minute','detail-five-minute','detail-hour']) {
      const p=read(name);const e=expected(facts(p,'local-model'))
      const bucket=p.data.groups[0].series[0].ts
      assert.equal(bucket%p.coverage.bucket_seconds,0)
      assert.deepEqual(series.toLatencySeries(p.data.groups),[{timestamp:new Date(bucket*1000).toISOString(),group:'latency',ttft_ms:Math.round(e.avg_ttft_ms!)}])
      assert.deepEqual(series.toSuccessRateSeries(p.data.groups),[{timestamp:new Date(bucket*1000).toISOString(),success_rate:100}])
      const nongroup=p.data.groups.find((g:any)=>g.group==='秘密组');assert.equal(nongroup.avg_ttft_ms,null)
      assert.deepEqual(series.toLatencySeries([nongroup]),[])
    }
    for (const value of [null,undefined,Number.NaN]) {assert.equal(format.formatLatency(value),'—');assert.equal(format.formatThroughput(value),'—');assert.equal(format.formatUptimePct(value),'—')}
  })
  await check('derived-series-order-missing-fields-and-status-weight', () => {
    // Deliberately shifted timestamps exercise conversion order, not historical Native facts.
    const original=detail.data.groups[0];const t=original.series[0].ts
    const derived=[{...original,series:[{...original.series[0],ts:t+120},{...original.series[0],ts:t}]}]
    assert.deepEqual(series.toLatencySeries(derived).map(p=>p.timestamp),[new Date(t*1000).toISOString(),new Date((t+120)*1000).toISOString()])
    const broken=structuredClone(full.data.models);delete broken[0].ttft_sum_us
    for (const metric of ['avg_latency_ms','avg_ttft_ms','avg_tps','success_rate'] as const) assert.equal(format.aggregatePerformanceMetric(broken,metric),null)
    const other=full.data.models.find((m:any)=>m.model_name==='other-model')
    // This DTO is reconstructed from actual summary counters and SQL group membership; not a captured detail response.
    assert.ok(samples.filter((r:any)=>r.model==='other-model').every((r:any)=>r.group==='allowed'))
    const grouped=status.aggregateStatusGroups([{modelName:'local-model',groups:detail.data.groups},{modelName:'other-model',groups:[{...other,group:'allowed',series:[]}]}])
    const actual=grouped.find(g=>g.group==='allowed')!
    const raw=[...facts(detail,'local-model','allowed'),...facts(full,'other-model','allowed')];const e=expected(raw)
    near(actual.avgLatencyMs,e.avg_latency_ms);near(actual.avgTps,e.avg_tps);near(actual.avgTtftMs,e.avg_ttft_ms);assert.equal(actual.modelCount,2)
  })
  await check('actual-summary-partial-missing-DOM', () => {
    assertSummary(render('summary-health-actual',<PerformanceHealthPanel/>,full))
    assertSummary(render('summary-overview-actual',<PerformanceOverview/>,full))
    const map=buildPerfMap(full.data.models,full);const item=map.get('local-model')!
    const dom=render('model-badge-actual',<ModelPerfBadge perf={item}/>,full)
    assert.ok(dom.text.includes('Partial performance coverage'));assert.ok(dom.document.querySelector('[aria-label*="1 missing samples"]'));dom.window.close()
  })
  await check('derived-summary-aggregate-priority-and-weighted-fallback', () => {
    // Model-list perturbation is an explicit precedence probe, not an actual Native response.
    const authoritative=structuredClone(full);authoritative.data.models=[authoritative.data.models[1]]
    assertSummary(render('health-aggregate-priority',<PerformanceHealthPanel/>,authoritative))
    assertSummary(render('overview-aggregate-priority',<PerformanceOverview/>,authoritative))
    const fallback=structuredClone(full);delete fallback.data.aggregate
    assertSummary(render('health-weighted-fallback',<PerformanceHealthPanel/>,fallback))
    assertSummary(render('overview-weighted-fallback',<PerformanceOverview/>,fallback))
  })
  await check('actual-detail-DOM-null-and-series-boundary', () => {
    const dom=render('detail-actual',<ModelDetailsPerformance model={{model_name:'local-model'} as any}/>,detail)
    const e=expected(facts(detail,'local-model'))
    assert.ok(dom.text.includes(Math.round(e.avg_latency_ms!)+'ms'))
    const row=Array.from(dom.document.querySelectorAll('tr')).find(r=>r.textContent?.includes('秘密组'))
    assert.ok(row?.textContent?.includes('—'),'nonstream TTFT must display unknown in group row')
    assert.ok(dom.text.includes('4 sampled requests; 0 missing samples'))
    const chart=dom.document.querySelector('[data-contract-chart="latency"]')!
    assert.deepEqual(JSON.parse(chart.textContent!),series.toLatencySeries(detail.data.groups));dom.window.close()
  })
  await check('actual-empty-partial-DOM-keeps-coverage', () => {
    assert.equal(empty.availability,'partial');assert.equal(empty.data.models.length,0)
    const health=render('empty-health-actual',<PerformanceHealthPanel/>,empty)
    assert.ok(health.text.includes('0 sampled requests; 0 missing samples'));health.window.close()
    const detailDom=render('empty-detail-actual',<ModelDetailsPerformance model={{model_name:'local-model'} as any}/>,emptyDetail)
    assert.ok(detailDom.text.includes('historical imports excluded'));detailDom.window.close()
    const overview=render('empty-overview-actual',<PerformanceOverview/>,empty)
    assert.ok(overview.text.includes('0 sampled requests; 0 missing samples'),'actual partial empty summary loses explicit coverage notice: '+overview.text)
    overview.window.close()
  })
},30000)
