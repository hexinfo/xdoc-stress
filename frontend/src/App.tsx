import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import './index.css'

interface ResultRow {
  firstRangeMs: number | null
  ts: number; fileName: string; steps: string; chain: string
  pageCount: number | null; uploadMs: number | null; prepareMs: number | null
  statePolls: number | null; stateReadyMs: number | null
  metricDownloadMs: number | null; metricConvertMs: number | null
  fullMs: number | null; e2eMs: number | null
  detail: string; success: boolean; error: string; warmup: boolean
}
interface RunStatus {
  running: boolean; stopRequested: boolean; logs: string[]; rows: ResultRow[]
  summary: { total: number; success: number; concurrency: number; wallMs: number; filesPerMin: number } | null
}
interface FileEntry { name: string; absPath: string; size: number }

const COLS = ['#', '文件', '页数', '上传', '就绪', '下载', '转换', '首页', '全页', '端到端', '状态']
const STEPS = ['upload', 'convert', 'range', 'excel']
const fmtSize = (n: number) => n >= 1048576 ? `${(n / 1048576).toFixed(1)}MB` : n >= 1024 ? `${(n / 1024).toFixed(1)}KB` : `${n}B`
const v = (x: number | null | undefined) => (x == null ? '—' : String(x))
const vt = (x: number | null | undefined) => x == null ? '—' : x >= 1000 ? (x / 1000).toFixed(1) + 's' : x + 'ms'

function StatusBadge({ status, sum }: { status: boolean; sum: NonNullable<RunStatus['summary']> }) {
  return (
    <span style={{
      font: '600 11px var(--mono)',
      padding: '2px 8px',
      borderRadius: 10,
      background: status ? 'var(--acc-bg)' : 'var(--ok-bg)',
      color: status ? 'var(--acc)' : sum.success === sum.total ? 'var(--ok)' : 'var(--bad)',
    }}>
      {status ? `运行中 ${sum.success}/${sum.total}` : `${sum.success}/${sum.total} · ${sum.filesPerMin} 件/min`}
    </span>
  )
}

/** 配置持久化：localStorage 保存/恢复 */
const CFG_KEY = 'xdoc-stress-config'
function saveConfig(cfg: Record<string, unknown>): void {
  try { localStorage.setItem(CFG_KEY, JSON.stringify(cfg)) } catch {}
}
function loadConfig(): Record<string, unknown> | null {
  try {
    const raw = localStorage.getItem(CFG_KEY)
    return raw ? JSON.parse(raw) : null
  } catch { return null }
}

export default function App() {
  const [files, setFiles] = useState<FileEntry[]>([])
  const [running, setRunning] = useState(false)
  const [status, setStatus] = useState<RunStatus | null>(null)
  const [logText, setLogText] = useState('')
  const logRef = useRef<HTMLDivElement>(null)
  const logSeen = useRef(0)
  const [baseUrl, setBaseUrl] = useState('')
  const [authToken, setAuthToken] = useState('')
  const [steps] = useState<string[]>(STEPS)
  const [concurrency, setConcurrency] = useState(1)
  const [repeats, setRepeats] = useState(1)
  const [rangeChunk, setRangeChunk] = useState(1048576)
  const [tileBatch, setTileBatch] = useState(10)
  const [pollInterval, setPollInterval] = useState(1000)
  const [pollMax, setPollMax] = useState(600)
  const [checked, setChecked] = useState<Set<string>>(new Set())

  // 首次挂载：恢复上次保存的配置
  useEffect(() => {
    const saved = loadConfig()
    if (!saved) return
    if (typeof saved.baseUrl === 'string') setBaseUrl(saved.baseUrl)
    if (typeof saved.authToken === 'string') setAuthToken(saved.authToken)
        if (typeof saved.concurrency === 'number') setConcurrency(saved.concurrency)
    if (typeof saved.repeats === 'number') setRepeats(saved.repeats)
    if (typeof saved.rangeChunk === 'number') setRangeChunk(saved.rangeChunk)
    if (typeof saved.tileBatch === 'number') setTileBatch(saved.tileBatch)
    if (typeof saved.pollIntervalMs === 'number') setPollInterval(saved.pollIntervalMs)
    if (typeof saved.pollMaxTimes === 'number') setPollMax(saved.pollMaxTimes)
  }, [])

  useEffect(() => {
    const t = setInterval(async () => {
      try {
        const s = await invoke<RunStatus>('get_status')
        setStatus(s); setRunning(s.running)
        if (s.logs.length > logSeen.current) {
          setLogText(p => p + s.logs.slice(logSeen.current).join('\n') + '\n')
          logSeen.current = s.logs.length
        } else if (s.logs.length < logSeen.current) {
          // Rust 端重置了日志（新一轮压测），同步重置
          logSeen.current = s.logs.length
          setLogText(s.logs.join('\n'))
        }
      } catch (e) { console.error('get_status failed:', e) }
    }, 1000)
    return () => clearInterval(t)
  }, [])
  useEffect(() => { if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight }, [logText])

  const addFiles = useCallback((paths: string[]) => {
    const nf = paths.map(p => ({ name: p.split('/').pop() ?? p, absPath: p, size: 0 }))
    setFiles(prev => { const m = new Map(prev.map(f => [f.name, f])); nf.forEach(f => m.set(f.name, f)); return [...m.values()] })
    setChecked(prev => { const s = new Set(prev); nf.forEach(f => s.add(f.absPath)); return s })
  }, [])

  const pickFolder = async () => {
    let dir: string | null = null
    try {
      dir = await open({ directory: true, multiple: false }) as string | null
    } catch (e) { alert('打开文件夹选择器失败: ' + String(e)); return }
    if (!dir) return
    try {
      const r = await invoke<{ name: string; path: string }[]>('list_dir_files', { dirPath: dir })
      const paths = (r ?? []).map(f => f.path)
      if (!paths.length) { alert('该文件夹没有可用的文件'); return }
      addFiles(paths)
    } catch (e) { alert(String(e)) }
  }
  const pickFiles = async () => {
    try {
      const sel = await open({ multiple: true }) as string[] | string | null
      if (!sel) return
      addFiles(Array.isArray(sel) ? sel : [sel])
    } catch (e) { alert('打开文件选择器失败: ' + String(e)) }
  }

  // 配置变化自动保存（跳过首次渲染，避免恢复前的空 state 覆盖存储）
  const restored = useRef(false)
  useEffect(() => {
    if (!restored.current) {
      restored.current = true
      return
    }
    saveConfig({
      baseUrl, authToken, steps, concurrency, repeats, rangeChunk, tileBatch,
      pollIntervalMs: pollInterval, pollMaxTimes: pollMax,
    })
  }, [baseUrl, authToken, steps, concurrency, repeats, rangeChunk, tileBatch, pollInterval, pollMax, files])

  const startRun = async () => {
    const sel = files.filter(f => checked.has(f.absPath)).map(f => f.absPath)
    if (!sel.length) return alert('请先添加并勾选文件')
    if (!baseUrl.trim()) return alert('请填 BASE_URL')
    setLogText(''); logSeen.current = 0
    try {
      await invoke('start_run', { config: { baseUrl: baseUrl.trim(), authToken: authToken.trim(), authHeaders: {}, steps, concurrency, repeats, rangeChunk, tileBatch, pollIntervalMs: pollInterval, pollMaxTimes: pollMax, files: sel } })
    } catch (e) { alert(String(e)) }
  }

  const rows = status?.rows ?? []
  const sum = status?.summary

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>

      <div className="main">
        <div className="p">
          <div className="p-h">文件<div className="sp" />
            <label className="ck" style={{ fontSize: 12 }}>
              <input type="checkbox" checked={files.length > 0 && checked.size === files.length} onChange={(e) => {
                setChecked(e.target.checked ? new Set(files.map((f) => f.absPath)) : new Set())
              }} />
              全选
            </label>
            <button className="b b-s" onClick={() => { setFiles([]); setChecked(new Set()) }}>清空</button>
          </div>
          <div style={{ display: 'flex', gap: 6, padding: '6px 10px', borderBottom: '1px solid var(--border-l)' }}>
            <button className="b" style={{ flex: 1, justifyContent: 'center' }} onClick={pickFolder}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>
      文件夹
    </button>
            <button className="b" style={{ flex: 1, justifyContent: 'center' }} onClick={pickFiles}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/></svg>
      文件
    </button>
          </div>
          <div style={{ flex: 1, overflowY: 'auto', padding: 4 }}>
            {files.length === 0 ? <div className="empty" style={{ padding: 20 }}>选择文件夹或文件</div> :
              files.map(f => (
                <label key={f.absPath} className="fi">
                  <input type="checkbox" checked={checked.has(f.absPath)} onChange={e => {
                    setChecked(prev => { const s = new Set(prev); if (e.target.checked) s.add(f.absPath); else s.delete(f.absPath); return s })
                  }} />
                  <span className="n">{f.name}</span>
                  
                </label>
              ))}
          </div>
          <div style={{ padding: '4px 12px', fontSize: 11, color: 'var(--txt-3)', borderTop: '1px solid var(--border-l)' }}>{files.length} 个文件</div>
        </div>

        <div className="mid">
          <div className="p" style={{ flex: 3, minHeight: 0 }}>
            <div className="p-h">结果{sum && (
              <StatusBadge status={running} sum={sum} />)}<div className="sp" />
              <button className="b b-s" onClick={() => {
                const lines = [COLS.join('\t')]
                rows.forEach((r, i) => lines.push([i+1, r.fileName, v(r.pageCount), vt(r.uploadMs), vt(r.stateReadyMs), v(r.metricDownloadMs), v(r.metricConvertMs), vt(r.fullMs), vt(r.e2eMs), r.success ? '✓' : '✗'].join('\t')))
                navigator.clipboard.writeText(lines.join('\n'))
              }}>复制</button>
              <button className="b b-s" onClick={async () => {
                const csv = await invoke<string>('export_csv')
                const a = document.createElement('a')
                a.href = URL.createObjectURL(new Blob([csv], { type: 'text/csv;charset=utf-8' }))
                a.download = 'core-flow.csv'; a.click()
              }}>CSV</button>
            </div>
            <div style={{ flex: 1, overflow: 'auto', minHeight: 0, height: 0 }}>
              {rows.length === 0 ? <div className="empty">配置参数后点「开始测试」</div> : (
                <table className="tb">
                  <thead><tr>{COLS.map(c => <th key={c}>{c}</th>)}</tr></thead>
                  <tbody>
                    {rows.map((r, i) => (
                      <tr key={i} className={r.success ? 'ok' : 'bad'}>
                        <td style={{ color: 'var(--txt-3)' }}>{i + 1}</td>
                        <td>{r.fileName.replace(/^core-\d+-\d+-/, '')}</td>
                        <td>{v(r.pageCount)}</td>
                        <td>{vt(r.uploadMs)}</td>
                        <td>{vt(r.stateReadyMs)}</td>
                        <td>{vt(r.metricDownloadMs)}</td>
                        <td>{vt(r.metricConvertMs)}</td>
                        <td>{vt(r.firstRangeMs)}</td>
                        <td>{vt(r.fullMs)}</td>
                        <td>{vt(r.e2eMs)}</td>
                        <td title={r.error}>{r.warmup ? '🔵' : r.success ? '✓' : <span style={{ color: 'var(--bad)' }}>✗ {r.error.slice(0, 20)}</span>}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          </div>
          <div className="p" style={{ flex: 1, minHeight: 0 }}>
            <div className="p-h">运行日志</div>
            <div ref={logRef} className="log" style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>{logText || '等待运行…'}</div>
          </div>
        </div>

        <div className="p">
          <div className="p-h">配置</div>
          <div style={{ flex: 1, overflowY: 'auto', padding: '8px 12px' }}>
            <div className="sec">目标</div>
            <label className="lbl">BASE_URL</label>
            <input className="in" value={baseUrl} onChange={e => setBaseUrl(e.target.value)} placeholder="http://host:8080/dvs" />
            <label className="lbl">AUTH_TOKEN</label>
            <input className="in" value={authToken} onChange={e => setAuthToken(e.target.value)} placeholder="留空则不带" />
            <div className="sec">并发</div>
            <div className="g2">
              <div><label className="lbl">线程</label><input className="in" type="number" value={concurrency} onChange={e => setConcurrency(+e.target.value)} min={1} /></div>
              <div><label className="lbl">轮次</label><input className="in" type="number" value={repeats} onChange={e => setRepeats(+e.target.value)} min={1} /></div>
            </div>
            <div className="sec">拉取</div>
            <div className="g2">
              <div><label className="lbl">分段 B</label><input className="in" type="number" value={rangeChunk} onChange={e => setRangeChunk(+e.target.value)} /></div>
              <div><label className="lbl">瓦片批</label><input className="in" type="number" value={tileBatch} onChange={e => setTileBatch(+e.target.value)} /></div>
            </div>
            <div className="sec">轮询</div>
            <div className="g2">
              <div><label className="lbl">间隔 ms</label><input className="in" type="number" value={pollInterval} onChange={e => setPollInterval(+e.target.value)} /></div>
              <div><label className="lbl">上限</label><input className="in" type="number" value={pollMax} onChange={e => setPollMax(+e.target.value)} /></div>
            </div>
            <div style={{ display: 'flex', gap: 8, marginTop: 16 }}>
              <button className="b b-p b-lg" onClick={startRun} disabled={running}>▶ 开始测试</button>
              <button className="b b-d b-lg" onClick={() => invoke('stop_run').catch(() => {})} disabled={!running}>■ 停止</button>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
