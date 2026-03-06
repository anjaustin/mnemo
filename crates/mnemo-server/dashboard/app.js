/* Mnemo Dashboard — Phase B: Feature Mapping */
'use strict';

// ─── Core helpers ──────────────────────────────────────────────────
const mnemo = {
  async api(method, path, body, timeoutMs) {
    const opts = { method, headers: {} };
    if (body) { opts.headers['Content-Type'] = 'application/json'; opts.body = JSON.stringify(body); }
    const ctrl = new AbortController();
    opts.signal = ctrl.signal;
    const timer = setTimeout(() => ctrl.abort(), timeoutMs || 30000);
    try {
      const res = await fetch(path, opts);
      clearTimeout(timer);
      if (!res.ok) {
        const err = await res.json().catch(() => ({ error: { message: res.statusText } }));
        throw new Error(err.error?.message || res.statusText);
      }
      const ct = res.headers.get('content-type') || '';
      if (ct.includes('application/json')) return res.json();
      return res.text();
    } catch (e) {
      clearTimeout(timer);
      if (e.name === 'AbortError') throw new Error('Request timed out');
      throw e;
    }
  },

  _intervals: [],
  poll(fn, intervalMs) {
    fn();
    const id = setInterval(fn, intervalMs);
    this._intervals.push(id);
    return id;
  },

  setText(id, text) { const el = document.getElementById(id); if (el) el.textContent = text; },
  setHtml(id, html) { const el = document.getElementById(id); if (el) el.innerHTML = html; },
  show(id) { const el = document.getElementById(id); if (el) el.classList.remove('hidden'); },
  hide(id) { const el = document.getElementById(id); if (el) el.classList.add('hidden'); },

  loading(id) { mnemo.setHtml(id, '<p class="muted">Loading...</p>'); mnemo.show(id); },
  error(id, msg) { mnemo.setHtml(id, `<p class="muted status-error">${escapeHtml(msg)}</p>`); mnemo.show(id); },
};

// ─── Confirmation modal ────────────────────────────────────────────
function confirmAction(message) {
  return new Promise(resolve => {
    const overlay = document.getElementById('modal-overlay');
    const body = document.getElementById('modal-body');
    const okBtn = document.getElementById('modal-ok');
    const cancelBtn = document.getElementById('modal-cancel');
    body.textContent = message;
    overlay.classList.remove('hidden');
    function cleanup(result) {
      overlay.classList.add('hidden');
      okBtn.removeEventListener('click', onOk);
      cancelBtn.removeEventListener('click', onCancel);
      resolve(result);
    }
    function onOk() { cleanup(true); }
    function onCancel() { cleanup(false); }
    okBtn.addEventListener('click', onOk);
    cancelBtn.addEventListener('click', onCancel);
  });
}

// ─── Utilities ─────────────────────────────────────────────────────
function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str == null ? '' : String(str);
  return div.innerHTML;
}

function fmtDate(iso) {
  if (!iso) return '--';
  const d = new Date(iso);
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

function fmtDateShort(iso) {
  if (!iso) return '--';
  const d = new Date(iso);
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

function toIso(localDatetimeStr) {
  if (!localDatetimeStr) return null;
  return new Date(localDatetimeStr).toISOString();
}

function statusBadge(ok, labelOk, labelFail) {
  return ok
    ? `<span class="status-ok">${escapeHtml(labelOk)}</span>`
    : `<span class="status-error">${escapeHtml(labelFail)}</span>`;
}

function truncId(id) { return id ? id.substring(0, 8) : '--'; }

// ─── Navigation ────────────────────────────────────────────────────
let currentPage = 'home';

function initNav() {
  const links = document.querySelectorAll('.nav-link');
  const pages = document.querySelectorAll('.page');

  window._navigate = function navigate(pageName) {
    currentPage = pageName;
    pages.forEach(p => p.classList.add('hidden'));
    links.forEach(l => l.classList.remove('active'));
    const page = document.getElementById('page-' + pageName);
    const link = document.querySelector(`[data-page="${pageName}"]`);
    if (page) page.classList.remove('hidden');
    if (link) link.classList.add('active');
  };

  links.forEach(link => {
    link.addEventListener('click', e => {
      e.preventDefault();
      const page = link.dataset.page;
      _navigate(page);
      history.pushState({ page }, '', link.href);
    });
  });

  window.addEventListener('popstate', () => {
    const page = location.pathname.replace(/^\/_\/?/, '').split('/')[0] || 'home';
    _navigate(page);
  });

  const path = location.pathname.replace(/^\/_\/?/, '').split('/')[0] || 'home';
  _navigate(path);
}

// ═══════════════════════════════════════════════════════════════════
// HOME PAGE
// ═══════════════════════════════════════════════════════════════════
function initHome() {
  mnemo.poll(async () => {
    try {
      const data = await mnemo.api('GET', '/health');
      mnemo.setText('card-status', data.status || 'ok');
      mnemo.setText('card-version', data.version || '--');
      mnemo.setText('nav-version', 'v' + (data.version || ''));
      mnemo.setText('health-label', data.status || 'ok');
      const dot = document.getElementById('health-dot');
      if (dot) dot.className = 'health-dot ' + (data.status === 'ok' ? 'ok' : 'error');
      const sc = document.getElementById('card-status');
      if (sc) sc.className = 'card-value ' + (data.status === 'ok' ? 'ok' : 'error');
    } catch (e) {
      mnemo.setText('health-label', 'offline');
      const dot = document.getElementById('health-dot');
      if (dot) dot.className = 'health-dot error';
    }
  }, 5000);

  mnemo.poll(async () => {
    try {
      const d = await mnemo.api('GET', '/api/v1/ops/summary');
      mnemo.setText('card-http-requests', String(d.http_requests_total || 0));
      mnemo.setText('card-webhooks', String(d.active_webhooks || 0));
      const dl = d.webhook_dead_letter_total || 0;
      const dlEl = document.getElementById('card-dead-letter');
      if (dlEl) { dlEl.textContent = String(dl); dlEl.className = 'card-value' + (dl > 0 ? ' warn' : ''); }
      const v = d.policy_violation_total || 0;
      const vEl = document.getElementById('card-violations');
      if (vEl) { vEl.textContent = String(v); vEl.className = 'card-value' + (v > 0 ? ' error' : ''); }
      // Recent activity
      const items = [];
      if (d.governance_audit_events_in_window > 0) items.push(`${d.governance_audit_events_in_window} governance events`);
      if (d.webhook_audit_events_in_window > 0) items.push(`${d.webhook_audit_events_in_window} webhook audit events`);
      if (d.pending_webhook_events > 0) items.push(`${d.pending_webhook_events} pending deliveries`);
      if (d.dead_letter_backlog > 0) items.push(`${d.dead_letter_backlog} dead-letter backlog`);
      if (d.http_responses_5xx > 0) items.push(`<span class="status-error">${d.http_responses_5xx} server errors</span>`);
      mnemo.setHtml('recent-activity-panel', items.length > 0
        ? '<ul>' + items.map(i => `<li>${i}</li>`).join('') + '</ul>'
        : '<p class="muted">No notable activity in window.</p>');
    } catch (e) { /* not critical */ }
  }, 10000);

  mnemo.poll(async () => {
    try {
      const data = await mnemo.api('GET', '/api/v1/memory/webhooks');
      const hooks = data.data || [];
      if (hooks.length === 0) {
        mnemo.setHtml('webhook-status-panel', '<p class="muted">No webhooks registered.</p>');
        return;
      }
      const rows = hooks.map(h => {
        const st = h.enabled ? '<span class="status-ok">enabled</span>' : '<span class="status-warn">disabled</span>';
        return `<tr>
          <td><code>${escapeHtml(truncId(h.id))}</code></td>
          <td>${escapeHtml(h.target_url)}</td>
          <td>${st}</td>
          <td>${escapeHtml(h.user_identifier)}</td>
          <td style="font-size:11px;color:var(--text-muted)">${escapeHtml((h.events||[]).join(', '))}</td>
        </tr>`;
      }).join('');
      mnemo.setHtml('webhook-status-panel', `<table>
        <thead><tr><th>ID</th><th>Target</th><th>Status</th><th>User</th><th>Events</th></tr></thead>
        <tbody>${rows}</tbody></table>`);
    } catch (e) {
      mnemo.setHtml('webhook-status-panel', '<p class="muted">Could not load webhooks.</p>');
    }
  }, 15000);
}

// ═══════════════════════════════════════════════════════════════════
// WEBHOOKS PAGE
// ═══════════════════════════════════════════════════════════════════
let webhooksData = [];

function initWebhooks() {
  loadWebhookGrid();
}

async function loadWebhookGrid() {
  mnemo.loading('webhooks-grid');
  try {
    const list = await mnemo.api('GET', '/api/v1/memory/webhooks');
    webhooksData = list.data || [];
    if (webhooksData.length === 0) {
      mnemo.setHtml('webhooks-grid', '<p class="muted">No webhooks registered.</p>');
      mnemo.hide('webhook-detail');
      return;
    }
    // Fetch stats for each webhook in parallel
    const statsPromises = webhooksData.map(h =>
      mnemo.api('GET', `/api/v1/memory/webhooks/${h.id}/stats`).catch(() => null)
    );
    const allStats = await Promise.all(statsPromises);

    const rows = webhooksData.map((h, i) => {
      const s = allStats[i] || {};
      const circuit = s.circuit_open ? '<span class="status-error">OPEN</span>' : '<span class="status-ok">closed</span>';
      return `<tr class="clickable-row" data-wh-id="${escapeHtml(h.id)}">
        <td><code>${escapeHtml(truncId(h.id))}</code></td>
        <td>${escapeHtml(h.target_url)}</td>
        <td>${h.enabled ? '<span class="status-ok">enabled</span>' : '<span class="status-warn">disabled</span>'}</td>
        <td>${circuit}</td>
        <td>${s.pending_events || 0}</td>
        <td>${s.dead_letter_events || 0}</td>
        <td>${s.delivered_events || 0}</td>
        <td>${escapeHtml(h.user_identifier)}</td>
      </tr>`;
    }).join('');

    mnemo.setHtml('webhooks-grid', `<table>
      <thead><tr><th>ID</th><th>Target</th><th>Status</th><th>Circuit</th><th>Pending</th><th>Dead</th><th>Delivered</th><th>User</th></tr></thead>
      <tbody>${rows}</tbody></table>`);

    // Click handler for rows
    document.querySelectorAll('.clickable-row[data-wh-id]').forEach(row => {
      row.addEventListener('click', () => loadWebhookDetail(row.dataset.whId));
    });
  } catch (e) {
    mnemo.error('webhooks-grid', 'Failed to load webhooks: ' + e.message);
  }
}

async function loadWebhookDetail(whId) {
  mnemo.show('webhook-detail');
  mnemo.loading('wh-detail-content');
  try {
    const [wh, stats, deadLetters, audit] = await Promise.all([
      mnemo.api('GET', `/api/v1/memory/webhooks/${whId}`),
      mnemo.api('GET', `/api/v1/memory/webhooks/${whId}/stats`),
      mnemo.api('GET', `/api/v1/memory/webhooks/${whId}/events/dead-letter?limit=50`),
      mnemo.api('GET', `/api/v1/memory/webhooks/${whId}/audit?limit=50`),
    ]);

    const circuit = stats.circuit_open
      ? `<span class="status-error">OPEN</span> (until ${fmtDate(stats.circuit_open_until)})`
      : '<span class="status-ok">closed</span>';

    // Dead-letter table
    const dlEvents = deadLetters.events || [];
    let dlRows = '';
    if (dlEvents.length > 0) {
      dlRows = dlEvents.map(ev => `<tr>
        <td><code>${escapeHtml(truncId(ev.id))}</code></td>
        <td>${escapeHtml(ev.event_type)}</td>
        <td>${ev.attempts}</td>
        <td style="font-size:11px">${escapeHtml(ev.last_error || '--')}</td>
        <td>${fmtDate(ev.created_at)}</td>
        <td><button class="btn btn-sm" onclick="retryEvent('${escapeHtml(whId)}','${escapeHtml(ev.id)}')">Retry</button></td>
      </tr>`).join('');
    }

    // Audit table
    const audits = audit.audit || [];
    let auditRows = '';
    if (audits.length > 0) {
      auditRows = audits.map(a => `<tr>
        <td>${escapeHtml(a.action)}</td>
        <td style="font-size:11px">${escapeHtml(a.request_id || '--')}</td>
        <td style="font-size:11px">${escapeHtml(JSON.stringify(a.details).substring(0, 80))}</td>
        <td>${fmtDate(a.at)}</td>
      </tr>`).join('');
    }

    mnemo.setHtml('wh-detail-content', `
      <div class="detail-header">
        <h2>Webhook ${escapeHtml(truncId(whId))}</h2>
        <div style="display:flex;gap:8px">
          <button class="btn btn-sm" onclick="replayWebhook('${escapeHtml(whId)}')">Replay All</button>
          <button class="btn btn-sm btn-danger" onclick="deleteWebhook('${escapeHtml(whId)}')">Delete</button>
        </div>
      </div>
      <div class="card-grid" style="margin-bottom:16px">
        <div class="card"><div class="card-label">Target</div><div style="font-size:12px;word-break:break-all">${escapeHtml(wh.target_url)}</div></div>
        <div class="card"><div class="card-label">Status</div><div class="card-value">${wh.enabled ? '<span class="ok">enabled</span>' : '<span class="status-warn">disabled</span>'}</div></div>
        <div class="card"><div class="card-label">Circuit</div><div class="card-value">${circuit}</div></div>
        <div class="card"><div class="card-label">Total</div><div class="card-value">${stats.total_events || 0}</div></div>
        <div class="card"><div class="card-label">Delivered</div><div class="card-value">${stats.delivered_events || 0}</div></div>
        <div class="card"><div class="card-label">Pending</div><div class="card-value">${stats.pending_events || 0}</div></div>
        <div class="card"><div class="card-label">Dead-Letter</div><div class="card-value${(stats.dead_letter_events||0) > 0 ? ' warn' : ''}">${stats.dead_letter_events || 0}</div></div>
      </div>

      <h2>Dead-Letter Queue</h2>
      ${dlEvents.length > 0
        ? `<table><thead><tr><th>ID</th><th>Type</th><th>Attempts</th><th>Error</th><th>Created</th><th>Action</th></tr></thead><tbody>${dlRows}</tbody></table>`
        : '<p class="muted">No dead-letter events.</p>'}

      <h2>Audit Log</h2>
      ${audits.length > 0
        ? `<table><thead><tr><th>Action</th><th>Request ID</th><th>Details</th><th>Time</th></tr></thead><tbody>${auditRows}</tbody></table>`
        : '<p class="muted">No audit records.</p>'}
    `);
  } catch (e) {
    mnemo.error('wh-detail-content', 'Failed to load webhook detail: ' + e.message);
  }
}

async function retryEvent(whId, eventId) {
  try {
    const res = await mnemo.api('POST', `/api/v1/memory/webhooks/${whId}/events/${eventId}/retry`, {});
    alert(res.queued ? 'Retry queued.' : ('Not queued: ' + res.reason));
    loadWebhookDetail(whId);
  } catch (e) { alert('Retry failed: ' + e.message); }
}

async function replayWebhook(whId) {
  if (!await confirmAction('Replay all events for this webhook?')) return;
  mnemo.loading('wh-detail-content');
  try {
    let cursor = null;
    let total = 0;
    for (let i = 0; i < 50; i++) {
      const url = `/api/v1/memory/webhooks/${whId}/events/replay?limit=100${cursor ? '&after_event_id=' + cursor : ''}`;
      const page = await mnemo.api('GET', url);
      total += (page.events || []).length;
      if (!page.next_after_event_id) break;
      cursor = page.next_after_event_id;
    }
    alert(`Replay scanned ${total} events.`);
    loadWebhookDetail(whId);
  } catch (e) { alert('Replay failed: ' + e.message); }
}

async function deleteWebhook(whId) {
  if (!await confirmAction('Permanently delete this webhook? This cannot be undone.')) return;
  try {
    await mnemo.api('DELETE', `/api/v1/memory/webhooks/${whId}`);
    mnemo.hide('webhook-detail');
    loadWebhookGrid();
  } catch (e) { alert('Delete failed: ' + e.message); }
}

// ═══════════════════════════════════════════════════════════════════
// RCA PAGE
// ═══════════════════════════════════════════════════════════════════
function initRca() {
  document.getElementById('rca-form').addEventListener('submit', async e => {
    e.preventDefault();
    const user = document.getElementById('rca-user').value.trim();
    const query = document.getElementById('rca-query').value.trim();
    const from = toIso(document.getElementById('rca-from').value);
    const to = toIso(document.getElementById('rca-to').value);
    const contract = document.getElementById('rca-contract').value;
    const policy = document.getElementById('rca-policy').value;
    if (!user || !query || !from || !to) { alert('User, query, from, and to are required.'); return; }

    mnemo.loading('rca-results');
    try {
      const data = await mnemo.api('POST', `/api/v1/memory/${encodeURIComponent(user)}/time_travel/trace`, {
        query, from, to, contract, retrieval_policy: policy
      });
      renderRcaResults(data);
    } catch (e) {
      mnemo.error('rca-results', 'Trace failed: ' + e.message);
    }
  });
}

function renderRcaResults(d) {
  const sf = d.snapshot_from || {};
  const st = d.snapshot_to || {};
  const gained = d.gained_facts || [];
  const lost = d.lost_facts || [];
  const timeline = d.timeline || [];
  const diag = d.retrieval_policy_diagnostics || {};

  let html = `
    <h2>Snapshot Comparison</h2>
    <div class="card-grid">
      <div class="card"><div class="card-label">FROM (${fmtDateShort(sf.as_of)})</div>
        <div>Facts: ${sf.fact_count || 0} &middot; Episodes: ${sf.episode_count || 0} &middot; Tokens: ${sf.token_count || 0}</div></div>
      <div class="card"><div class="card-label">TO (${fmtDateShort(st.as_of)})</div>
        <div>Facts: ${st.fact_count || 0} &middot; Episodes: ${st.episode_count || 0} &middot; Tokens: ${st.token_count || 0}</div></div>
    </div>

    <h2>Gained Facts (${gained.length})</h2>
    ${gained.length > 0 ? '<ul>' + gained.map(f => `<li>${escapeHtml(f.fact || f.text || JSON.stringify(f))}</li>`).join('') + '</ul>' : '<p class="muted">None.</p>'}

    <h2>Lost Facts (${lost.length})</h2>
    ${lost.length > 0 ? '<ul class="status-error">' + lost.map(f => `<li>${escapeHtml(f.fact || f.text || JSON.stringify(f))}</li>`).join('') + '</ul>' : '<p class="muted">None.</p>'}

    <h2>Timeline (${timeline.length} events)</h2>
    ${timeline.length > 0 ? `<table><thead><tr><th>Time</th><th>Type</th><th>Description</th></tr></thead><tbody>
      ${timeline.map(t => `<tr><td>${fmtDate(t.at)}</td><td><code>${escapeHtml(t.event_type)}</code></td><td>${escapeHtml(t.description)}</td></tr>`).join('')}
    </tbody></table>` : '<p class="muted">No timeline events.</p>'}

    <h2>Retrieval Policy Diagnostics</h2>
    <div class="card-grid">
      <div class="card"><div class="card-label">Max Tokens</div><div class="card-value">${diag.effective_max_tokens || '--'}</div></div>
      <div class="card"><div class="card-label">Min Relevance</div><div class="card-value">${diag.effective_min_relevance != null ? diag.effective_min_relevance.toFixed(2) : '--'}</div></div>
      <div class="card"><div class="card-label">Temporal Intent</div><div class="card-value" style="font-size:16px">${escapeHtml(diag.effective_temporal_intent || '--')}</div></div>
      <div class="card"><div class="card-label">Temporal Weight</div><div class="card-value">${diag.effective_temporal_weight != null ? diag.effective_temporal_weight.toFixed(2) : '--'}</div></div>
    </div>

    <h2>Summary</h2>
    <div class="panel"><p>${escapeHtml(d.summary || '--')}</p></div>
    <p style="font-size:11px;color:var(--text-muted)">Contract: ${escapeHtml(d.contract_applied)} &middot; Policy: ${escapeHtml(d.retrieval_policy_applied)}</p>
  `;
  mnemo.setHtml('rca-results', html);
  mnemo.show('rca-results');
}

// ═══════════════════════════════════════════════════════════════════
// GOVERNANCE PAGE
// ═══════════════════════════════════════════════════════════════════
let govCurrentUser = null;

function initGovernance() {
  document.getElementById('gov-load-btn').addEventListener('click', () => {
    const user = document.getElementById('gov-user').value.trim();
    if (!user) { alert('Enter a username.'); return; }
    govCurrentUser = user;
    loadGovernance(user);
  });
}

async function loadGovernance(user) {
  mnemo.loading('gov-policy-panel');
  mnemo.hide('gov-violations-panel');
  mnemo.hide('gov-audit-panel');
  try {
    const data = await mnemo.api('GET', `/api/v1/policies/${encodeURIComponent(user)}`);
    const p = data.policy;
    mnemo.setHtml('gov-policy-panel', `
      <div class="detail-header">
        <h2>Policy for ${escapeHtml(p.user_identifier)}</h2>
        <div style="display:flex;gap:8px">
          <button class="btn btn-sm" id="gov-edit-btn">Edit</button>
          <button class="btn btn-sm" id="gov-preview-btn">Preview Impact</button>
        </div>
      </div>
      <table>
        <tr><td style="color:var(--text-muted)">Retention (messages)</td><td>${p.retention_days_message} days</td></tr>
        <tr><td style="color:var(--text-muted)">Retention (text)</td><td>${p.retention_days_text} days</td></tr>
        <tr><td style="color:var(--text-muted)">Retention (JSON)</td><td>${p.retention_days_json} days</td></tr>
        <tr><td style="color:var(--text-muted)">Domain Allowlist</td><td>${escapeHtml((p.webhook_domain_allowlist||[]).join(', ') || '(any)')}</td></tr>
        <tr><td style="color:var(--text-muted)">Default Contract</td><td>${escapeHtml(p.default_memory_contract)}</td></tr>
        <tr><td style="color:var(--text-muted)">Default Policy</td><td>${escapeHtml(p.default_retrieval_policy)}</td></tr>
        <tr><td style="color:var(--text-muted)">Updated</td><td>${fmtDate(p.updated_at)}</td></tr>
      </table>
      <div id="gov-edit-form" class="hidden" style="margin-top:16px">
        <div class="form-row">
          <label>Retention Messages (days)<input type="number" id="gov-ret-msg" value="${p.retention_days_message}"></label>
          <label>Retention Text (days)<input type="number" id="gov-ret-txt" value="${p.retention_days_text}"></label>
          <label>Retention JSON (days)<input type="number" id="gov-ret-json" value="${p.retention_days_json}"></label>
        </div>
        <div class="form-row">
          <label>Domain Allowlist (comma-separated)<input type="text" id="gov-allowlist" value="${escapeHtml((p.webhook_domain_allowlist||[]).join(', '))}"></label>
        </div>
        <div class="form-row">
          <label>Default Contract
            <select id="gov-contract">
              ${['default','support_safe','current_strict','historical_strict'].map(c => `<option value="${c}"${c===p.default_memory_contract?' selected':''}>${c}</option>`).join('')}
            </select>
          </label>
          <label>Default Policy
            <select id="gov-policy-select">
              ${['balanced','precision','recall','stability'].map(c => `<option value="${c}"${c===p.default_retrieval_policy?' selected':''}>${c}</option>`).join('')}
            </select>
          </label>
          <button type="button" class="btn" id="gov-save-btn">Save</button>
        </div>
      </div>
    `);
    mnemo.show('gov-policy-panel');

    document.getElementById('gov-edit-btn').addEventListener('click', () => {
      const form = document.getElementById('gov-edit-form');
      form.classList.toggle('hidden');
    });

    document.getElementById('gov-save-btn').addEventListener('click', () => saveGovernance(user));
    document.getElementById('gov-preview-btn').addEventListener('click', () => previewGovernance(user));

    // Load violations and audit
    loadViolations(user);
    loadGovernanceAudit(user);
  } catch (e) {
    mnemo.error('gov-policy-panel', 'Failed to load policy: ' + e.message);
  }
}

function getGovFormData() {
  const al = document.getElementById('gov-allowlist').value.trim();
  return {
    retention_days_message: parseInt(document.getElementById('gov-ret-msg').value) || 90,
    retention_days_text: parseInt(document.getElementById('gov-ret-txt').value) || 90,
    retention_days_json: parseInt(document.getElementById('gov-ret-json').value) || 90,
    webhook_domain_allowlist: al ? al.split(',').map(s => s.trim()).filter(Boolean) : [],
    default_memory_contract: document.getElementById('gov-contract').value,
    default_retrieval_policy: document.getElementById('gov-policy-select').value,
  };
}

async function saveGovernance(user) {
  try {
    await mnemo.api('PUT', `/api/v1/policies/${encodeURIComponent(user)}`, getGovFormData());
    alert('Policy saved.');
    loadGovernance(user);
  } catch (e) { alert('Save failed: ' + e.message); }
}

async function previewGovernance(user) {
  const editForm = document.getElementById('gov-edit-form');
  if (editForm && editForm.classList.contains('hidden')) {
    editForm.classList.remove('hidden');
    alert('Edit the fields first, then click Preview Impact.');
    return;
  }
  try {
    const res = await mnemo.api('POST', `/api/v1/policies/${encodeURIComponent(user)}/preview`, getGovFormData());
    alert(`Preview: ~${res.estimated_affected_episodes_total} episodes affected (${res.confidence})\n` +
      `  Messages: ~${res.estimated_affected_message_episodes}\n` +
      `  Text: ~${res.estimated_affected_text_episodes}\n` +
      `  JSON: ~${res.estimated_affected_json_episodes}`);
  } catch (e) { alert('Preview failed: ' + e.message); }
}

async function loadViolations(user) {
  mnemo.loading('gov-violations-panel');
  try {
    const now = new Date();
    const from = new Date(now.getTime() - 24 * 60 * 60 * 1000).toISOString();
    const data = await mnemo.api('GET', `/api/v1/policies/${encodeURIComponent(user)}/violations?from=${from}&to=${now.toISOString()}&limit=50`);
    const viols = data.violations || [];
    if (viols.length === 0) {
      mnemo.setHtml('gov-violations-panel', '<h2>Violations (24h)</h2><p class="muted">None.</p>');
    } else {
      const rows = viols.map(v => `<tr>
        <td>${escapeHtml(v.action)}</td>
        <td style="font-size:11px">${escapeHtml(v.request_id || '--')}</td>
        <td>${fmtDate(v.at)}</td>
      </tr>`).join('');
      mnemo.setHtml('gov-violations-panel', `<h2>Violations (24h) — ${viols.length}</h2>
        <table><thead><tr><th>Action</th><th>Request ID</th><th>Time</th></tr></thead><tbody>${rows}</tbody></table>`);
    }
    mnemo.show('gov-violations-panel');
  } catch (e) {
    mnemo.error('gov-violations-panel', 'Could not load violations.');
  }
}

async function loadGovernanceAudit(user) {
  mnemo.loading('gov-audit-panel');
  try {
    const data = await mnemo.api('GET', `/api/v1/policies/${encodeURIComponent(user)}/audit?limit=50`);
    const audits = data.audit || [];
    if (audits.length === 0) {
      mnemo.setHtml('gov-audit-panel', '<h2>Audit Trail</h2><p class="muted">No records.</p>');
    } else {
      const rows = audits.map(a => `<tr>
        <td>${escapeHtml(a.action)}</td>
        <td style="font-size:11px">${escapeHtml(JSON.stringify(a.details).substring(0, 100))}</td>
        <td>${fmtDate(a.at)}</td>
      </tr>`).join('');
      mnemo.setHtml('gov-audit-panel', `<h2>Audit Trail — ${audits.length}</h2>
        <table><thead><tr><th>Action</th><th>Details</th><th>Time</th></tr></thead><tbody>${rows}</tbody></table>`);
    }
    mnemo.show('gov-audit-panel');
  } catch (e) {
    mnemo.error('gov-audit-panel', 'Could not load audit trail.');
  }
}

// ═══════════════════════════════════════════════════════════════════
// TRACES PAGE
// ═══════════════════════════════════════════════════════════════════
function initTraces() {
  document.getElementById('trace-lookup-btn').addEventListener('click', async () => {
    const reqId = document.getElementById('trace-request-id').value.trim();
    if (!reqId) { alert('Enter a request ID.'); return; }
    mnemo.loading('trace-results');
    try {
      const data = await mnemo.api('GET', `/api/v1/traces/${encodeURIComponent(reqId)}`);
      renderTraceResults(data);
    } catch (e) {
      mnemo.error('trace-results', 'Trace lookup failed: ' + e.message);
    }
  });
}

function renderTraceResults(d) {
  const eps = d.matched_episodes || [];
  const whEvts = d.matched_webhook_events || [];
  const whAudit = d.matched_webhook_audit || [];
  const govAudit = d.matched_governance_audit || [];
  const sum = d.summary || {};

  let html = `<h2>Trace: ${escapeHtml(d.request_id)}</h2>
    <div class="card-grid" style="margin-bottom:16px">
      <div class="card"><div class="card-label">Episodes</div><div class="card-value">${eps.length}</div></div>
      <div class="card"><div class="card-label">Webhook Events</div><div class="card-value">${whEvts.length}</div></div>
      <div class="card"><div class="card-label">Webhook Audit</div><div class="card-value">${whAudit.length}</div></div>
      <div class="card"><div class="card-label">Governance Audit</div><div class="card-value">${govAudit.length}</div></div>
    </div>`;

  if (eps.length > 0) {
    html += `<h2>Episodes</h2><table><thead><tr><th>Episode ID</th><th>Session</th><th>Preview</th><th>Created</th></tr></thead><tbody>
      ${eps.map(e => `<tr><td><code>${escapeHtml(truncId(e.episode_id))}</code></td><td><code>${escapeHtml(truncId(e.session_id))}</code></td><td>${escapeHtml(e.preview)}</td><td>${fmtDate(e.created_at)}</td></tr>`).join('')}
    </tbody></table>`;
  }

  if (whEvts.length > 0) {
    html += `<h2>Webhook Events</h2><table><thead><tr><th>ID</th><th>Type</th><th>Delivered</th><th>Dead Letter</th><th>Created</th></tr></thead><tbody>
      ${whEvts.map(e => `<tr><td><code>${escapeHtml(truncId(e.id))}</code></td><td>${escapeHtml(e.event_type)}</td><td>${statusBadge(e.delivered,'yes','no')}</td><td>${statusBadge(!e.dead_letter,'no','yes')}</td><td>${fmtDate(e.created_at)}</td></tr>`).join('')}
    </tbody></table>`;
  }

  if (whAudit.length > 0) {
    html += `<h2>Webhook Audit</h2><table><thead><tr><th>Action</th><th>Details</th><th>Time</th></tr></thead><tbody>
      ${whAudit.map(a => `<tr><td>${escapeHtml(a.action)}</td><td style="font-size:11px">${escapeHtml(JSON.stringify(a.details).substring(0,100))}</td><td>${fmtDate(a.at)}</td></tr>`).join('')}
    </tbody></table>`;
  }

  if (govAudit.length > 0) {
    html += `<h2>Governance Audit</h2><table><thead><tr><th>Action</th><th>Details</th><th>Time</th></tr></thead><tbody>
      ${govAudit.map(a => `<tr><td>${escapeHtml(a.action)}</td><td style="font-size:11px">${escapeHtml(JSON.stringify(a.details).substring(0,100))}</td><td>${fmtDate(a.at)}</td></tr>`).join('')}
    </tbody></table>`;
  }

  mnemo.setHtml('trace-results', html);
  mnemo.show('trace-results');
}

// ═══════════════════════════════════════════════════════════════════
// EXPLORER PAGE
// ═══════════════════════════════════════════════════════════════════
let graphData = null;

function initExplorer() {
  document.getElementById('explorer-load-btn').addEventListener('click', async () => {
    const user = document.getElementById('explorer-user').value.trim();
    if (!user) { alert('Enter a username.'); return; }
    mnemo.loading('explorer-entities');
    mnemo.hide('explorer-graph');
    try {
      // Resolve username to user_id (UUID)
      const userObj = await mnemo.api('GET', `/api/v1/users/external/${encodeURIComponent(user)}`);
      const userId = userObj.id;
      if (!userId) throw new Error('Could not resolve user ID.');
      const data = await mnemo.api('GET', `/api/v1/users/${userId}/entities`);
      const entities = data.data || [];
      if (entities.length === 0) {
        mnemo.setHtml('explorer-entities', '<p class="muted">No entities found for this user.</p>');
        return;
      }
      const rows = entities.map(e => `<tr class="clickable-row" data-entity-id="${escapeHtml(e.id)}">
        <td><code>${escapeHtml(truncId(e.id))}</code></td>
        <td>${escapeHtml(e.name)}</td>
        <td>${escapeHtml(e.entity_type || '--')}</td>
        <td style="font-size:11px">${escapeHtml(e.summary || '--')}</td>
      </tr>`).join('');
      mnemo.setHtml('explorer-entities', `<h2>Entities (${entities.length})</h2>
        <table><thead><tr><th>ID</th><th>Name</th><th>Type</th><th>Summary</th></tr></thead>
        <tbody>${rows}</tbody></table>
        <p class="muted" style="margin-top:8px">Click an entity to load its subgraph.</p>`);
      mnemo.show('explorer-entities');

      document.querySelectorAll('.clickable-row[data-entity-id]').forEach(row => {
        row.addEventListener('click', () => loadSubgraph(row.dataset.entityId));
      });
    } catch (e) {
      mnemo.error('explorer-entities', 'Failed to load entities: ' + e.message);
    }
  });
}

async function loadSubgraph(entityId) {
  mnemo.show('explorer-graph');
  try {
    graphData = await mnemo.api('GET', `/api/v1/entities/${entityId}/subgraph?depth=2&max_nodes=50`);
    renderGraph(graphData);
  } catch (e) {
    mnemo.setHtml('explorer-graph', `<p class="muted status-error">Failed to load subgraph: ${escapeHtml(e.message)}</p>`);
  }
}

function renderGraph(data) {
  const canvas = document.getElementById('graph-canvas');
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  const W = canvas.width = canvas.clientWidth;
  const H = canvas.height = canvas.clientHeight || 500;

  const nodes = (data.nodes || []).map((n, i) => ({
    id: n.entity.id,
    label: n.entity.name || truncId(n.entity.id),
    type: n.entity.entity_type || 'unknown',
    depth: n.depth,
    x: W / 2 + (Math.random() - 0.5) * W * 0.6,
    y: H / 2 + (Math.random() - 0.5) * H * 0.6,
    vx: 0, vy: 0,
  }));
  const nodeMap = {};
  nodes.forEach(n => nodeMap[n.id] = n);

  const edges = (data.edges || []).filter(e => nodeMap[e.source_entity_id] && nodeMap[e.target_entity_id]).map(e => ({
    source: nodeMap[e.source_entity_id],
    target: nodeMap[e.target_entity_id],
    label: e.label || '',
    fact: e.fact || '',
    invalid: !!e.invalid_at,
  }));

  if (nodes.length === 0) {
    ctx.clearRect(0, 0, W, H);
    ctx.fillStyle = '#6b7280';
    ctx.font = '14px monospace';
    ctx.textAlign = 'center';
    ctx.fillText('No nodes in subgraph.', W / 2, H / 2);
    return;
  }

  // Simple force-directed simulation
  const ITERATIONS = 120;
  const REPULSION = 3000;
  const SPRING = 0.01;
  const REST = 100;
  const DAMPING = 0.85;

  for (let iter = 0; iter < ITERATIONS; iter++) {
    // Repulsion between all node pairs
    for (let i = 0; i < nodes.length; i++) {
      for (let j = i + 1; j < nodes.length; j++) {
        let dx = nodes[j].x - nodes[i].x;
        let dy = nodes[j].y - nodes[i].y;
        let dist = Math.sqrt(dx * dx + dy * dy) || 1;
        let force = REPULSION / (dist * dist);
        let fx = (dx / dist) * force;
        let fy = (dy / dist) * force;
        nodes[i].vx -= fx; nodes[i].vy -= fy;
        nodes[j].vx += fx; nodes[j].vy += fy;
      }
    }
    // Spring attraction along edges
    for (const e of edges) {
      let dx = e.target.x - e.source.x;
      let dy = e.target.y - e.source.y;
      let dist = Math.sqrt(dx * dx + dy * dy) || 1;
      let force = SPRING * (dist - REST);
      let fx = (dx / dist) * force;
      let fy = (dy / dist) * force;
      e.source.vx += fx; e.source.vy += fy;
      e.target.vx -= fx; e.target.vy -= fy;
    }
    // Apply velocity, damping, boundary
    for (const n of nodes) {
      n.vx *= DAMPING; n.vy *= DAMPING;
      n.x += n.vx; n.y += n.vy;
      n.x = Math.max(40, Math.min(W - 40, n.x));
      n.y = Math.max(40, Math.min(H - 40, n.y));
    }
  }

  // Draw
  ctx.clearRect(0, 0, W, H);

  // Edges
  for (const e of edges) {
    ctx.beginPath();
    ctx.moveTo(e.source.x, e.source.y);
    ctx.lineTo(e.target.x, e.target.y);
    if (e.invalid) {
      ctx.setLineDash([4, 4]);
      ctx.strokeStyle = 'rgba(239,68,68,0.4)';
    } else {
      ctx.setLineDash([]);
      ctx.strokeStyle = 'rgba(99,102,241,0.4)';
    }
    ctx.lineWidth = 1.5;
    ctx.stroke();
    ctx.setLineDash([]);
    // Edge label
    if (e.label) {
      const mx = (e.source.x + e.target.x) / 2;
      const my = (e.source.y + e.target.y) / 2;
      ctx.fillStyle = '#6b7280';
      ctx.font = '9px monospace';
      ctx.textAlign = 'center';
      ctx.fillText(e.label.substring(0, 20), mx, my - 4);
    }
  }

  // Nodes
  const typeColors = { person: '#6366f1', location: '#22c55e', organization: '#eab308', concept: '#818cf8' };
  for (const n of nodes) {
    const r = n.depth === 0 ? 10 : 7;
    ctx.beginPath();
    ctx.arc(n.x, n.y, r, 0, Math.PI * 2);
    ctx.fillStyle = typeColors[n.type.toLowerCase()] || '#9ca3af';
    ctx.fill();
    ctx.strokeStyle = '#e1e4ed';
    ctx.lineWidth = n.depth === 0 ? 2 : 1;
    ctx.stroke();
    // Label
    ctx.fillStyle = '#e1e4ed';
    ctx.font = (n.depth === 0 ? 'bold ' : '') + '11px monospace';
    ctx.textAlign = 'center';
    ctx.fillText(n.label.substring(0, 18), n.x, n.y + r + 14);
  }

  // Legend
  ctx.fillStyle = '#6b7280';
  ctx.font = '10px monospace';
  ctx.textAlign = 'left';
  ctx.fillText(`${nodes.length} nodes, ${edges.length} edges`, 8, H - 8);
}

// ═══════════════════════════════════════════════════════════════════
// BOOT
// ═══════════════════════════════════════════════════════════════════
document.addEventListener('DOMContentLoaded', () => {
  initNav();
  initHome();
  initWebhooks();
  initRca();
  initGovernance();
  initTraces();
  initExplorer();
});
