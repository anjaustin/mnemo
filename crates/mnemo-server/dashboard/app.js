/* Mnemo Dashboard — Phase C: Face-Melting Polish */
'use strict';

// ─── Core API helper ───────────────────────────────────────────────
const mnemo = {
  async api(method, path, body, timeoutMs) {
    const opts = { method, headers: {} };
    if (body !== undefined && body !== null) {
      opts.headers['Content-Type'] = 'application/json';
      opts.body = JSON.stringify(body);
    }
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

  loading(id) {
    const el = document.getElementById(id);
    if (!el) return;
    el.innerHTML = `
      <div class="skeleton-block" style="height:18px;width:60%;margin-bottom:8px"></div>
      <div class="skeleton-block" style="height:18px;width:80%;margin-bottom:8px"></div>
      <div class="skeleton-block" style="height:18px;width:50%"></div>`;
    el.classList.remove('hidden');
  },
  error(id, msg) {
    const el = document.getElementById(id);
    if (!el) return;
    el.innerHTML = `<p class="muted" style="color:var(--red)">${escapeHtml(msg)}</p>`;
    el.classList.remove('hidden');
  },
};

// ─── Toast system ──────────────────────────────────────────────────
const toast = (() => {
  function show(type, title, msg) {
    const container = document.getElementById('toast-container');
    if (!container) return;
    const el = document.createElement('div');
    el.className = `toast ${type}`;
    el.innerHTML = `
      <div class="toast-title">${escapeHtml(title)}</div>
      ${msg ? `<div class="toast-msg">${escapeHtml(msg)}</div>` : ''}
      <button class="toast-close" aria-label="Close">&times;</button>`;
    container.appendChild(el);
    requestAnimationFrame(() => el.classList.add('toast-visible'));
    const dismiss = () => {
      el.classList.remove('toast-visible');
      el.classList.add('toast-out');
      // Use setTimeout fallback — animationend is unreliable when tab is backgrounded
      setTimeout(() => el.remove(), 350);
    };
    el.querySelector('.toast-close').addEventListener('click', dismiss);
    setTimeout(dismiss, type === 'error' ? 8000 : 4000);
  }
  return {
    success: (title, msg) => show('success', title, msg),
    error:   (title, msg) => show('error', title, msg),
    info:    (title, msg) => show('info', title, msg),
    warn:    (title, msg) => show('warn', title, msg),
  };
})();

// ─── Confirmation modal ────────────────────────────────────────────
function confirmAction(message, title) {
  return new Promise(resolve => {
    const overlay = document.getElementById('modal-overlay');
    const titleEl = document.getElementById('modal-title');
    const body = document.getElementById('modal-body');
    const okBtn = document.getElementById('modal-ok');
    const cancelBtn = document.getElementById('modal-cancel');
    if (titleEl) titleEl.textContent = title || 'Confirm';
    body.textContent = message;
    overlay.classList.remove('hidden');
    function cleanup(result) {
      overlay.classList.add('hidden');
      okBtn.removeEventListener('click', onOk);
      cancelBtn.removeEventListener('click', onCancel);
      overlay.removeEventListener('click', onBackdrop);
      resolve(result);
    }
    function onOk() { cleanup(true); }
    function onCancel() { cleanup(false); }
    function onBackdrop(e) { if (e.target === overlay) cleanup(false); }
    okBtn.addEventListener('click', onOk);
    cancelBtn.addEventListener('click', onCancel);
    overlay.addEventListener('click', onBackdrop);
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

function fmtDateAgo(iso) {
  if (!iso) return '--';
  const diff = Date.now() - new Date(iso).getTime();
  if (diff < 0) return 'just now';
  const s = Math.floor(diff / 1000);
  if (s < 60) return s + 's ago';
  const m = Math.floor(s / 60);
  if (m < 60) return m + 'm ago';
  const h = Math.floor(m / 60);
  if (h < 24) return h + 'h ago';
  return Math.floor(h / 24) + 'd ago';
}

function toIso(localDatetimeStr) {
  if (!localDatetimeStr) return null;
  return new Date(localDatetimeStr).toISOString();
}

function badge(label, type) {
  // type: green | yellow | red | blue | gray
  const t = type || 'gray';
  return `<span class="badge badge-${t}">${escapeHtml(String(label))}</span>`;
}

function truncId(id) { return id ? id.substring(0, 8) : '--'; }

function dashboardSegments() {
  return location.pathname.replace(/^\/_\/?/, '').split('/').filter(Boolean);
}

function dashboardPageFromPath(pathname) {
  return pathname.replace(/^\/_\/?/, '').split('/')[0] || 'home';
}

function dashboardQuery() {
  return new URLSearchParams(location.search);
}

function traceHref(requestId) {
  return requestId ? `/_/traces/${encodeURIComponent(requestId)}` : '';
}

function traceLink(requestId, label) {
  if (!requestId) return '--';
  const href = traceHref(requestId);
  const text = label || ('req ' + truncId(requestId));
  return `<a class="link" href="${href}" data-trace-link="${escapeHtml(requestId)}">${escapeHtml(text)}</a>`;
}

function syncPageFromPath(pageName) {
  const segments = dashboardSegments();

  if (pageName === 'webhooks') {
    const whId = segments[1] ? decodeURIComponent(segments[1]) : null;
    _selectedWebhookId = whId;
    if (whId) {
      openWebhookDetail(whId, false);
    } else {
      mnemo.hide('webhook-detail');
      document.querySelectorAll('.clickable-row[data-wh-id]').forEach(r => r.classList.remove('selected-row'));
    }
    return;
  }

  if (pageName === 'governance') {
    const user = segments[1] ? decodeURIComponent(segments[1]) : null;
    if (user && user !== govCurrentUser) {
      const input = document.getElementById('gov-user');
      if (input) input.value = user;
      govCurrentUser = user;
      loadGovernance(user);
    }
    return;
  }

  if (pageName === 'traces') {
    const reqId = segments[1] ? decodeURIComponent(segments[1]) : null;
    const input = document.getElementById('trace-request-id');
    if (reqId && input && input.value !== reqId) {
      input.value = reqId;
      setTimeout(() => document.getElementById('trace-lookup-btn')?.click(), 0);
    }
  }
}

// ─── Navigation with lazy init ─────────────────────────────────────
let currentPage = 'home';

const _pageInits = {};

function initNav() {
  const links = document.querySelectorAll('.nav-link');
  const pages = document.querySelectorAll('.page');

  window._navigate = function navigate(pageName, pushUrl, exactPath) {
    currentPage = pageName;
    pages.forEach(p => p.classList.add('hidden'));
    links.forEach(l => { l.classList.remove('active'); l.removeAttribute('aria-current'); });
    const page = document.getElementById('page-' + pageName);
    const link = document.querySelector(`[data-page="${pageName}"]`);
    if (page) {
      page.classList.remove('hidden');
      page.classList.add('page-in');
      page.addEventListener('animationend', () => page.classList.remove('page-in'), { once: true });
    }
    if (link) {
      link.classList.add('active');
      link.setAttribute('aria-current', 'page');
    }

    // Always update URL (unless explicitly suppressed for popstate handler)
    if (pushUrl !== false) {
      const url = exactPath || (pageName === 'home' ? '/_/' : `/_/${pageName}`);
      history.pushState({ page: pageName }, '', url);
    }

    // Lazy init
    if (_pageInits[pageName] && !_pageInits[pageName]._done) {
      _pageInits[pageName]._done = true;
      _pageInits[pageName].init();
    }

    syncPageFromPath(pageName);
  };

  links.forEach(link => {
    link.addEventListener('click', e => {
      e.preventDefault();
      _navigate(link.dataset.page);
    });
  });

  window.addEventListener('popstate', () => {
    const page = dashboardPageFromPath(location.pathname);
    _navigate(page, false);
  });

  const path = dashboardPageFromPath(location.pathname);
  _navigate(path, false);
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

      // Recent activity feed
      const items = [];
      if (d.governance_audit_events_in_window > 0)
        items.push({ text: `${d.governance_audit_events_in_window} governance events`, type: 'info' });
      if (d.webhook_audit_events_in_window > 0)
        items.push({ text: `${d.webhook_audit_events_in_window} webhook audit events`, type: 'info' });
      if (d.pending_webhook_events > 0)
        items.push({ text: `${d.pending_webhook_events} pending deliveries`, type: 'warn' });
      if (d.dead_letter_backlog > 0)
        items.push({ text: `${d.dead_letter_backlog} dead-letter backlog`, type: 'error' });
      if (d.http_responses_5xx > 0)
        items.push({ text: `${d.http_responses_5xx} server errors`, type: 'error' });

      if (items.length > 0) {
        mnemo.setHtml('recent-activity-panel',
          items.map(item => `<div class="activity-item activity-${item.type}">${escapeHtml(item.text)}</div>`).join(''));
      } else {
        mnemo.setHtml('recent-activity-panel', '<p class="muted">No notable activity in window.</p>');
      }
    } catch (e) { /* not critical */ }
  }, 10000);

  mnemo.poll(async () => {
    try {
      const data = await mnemo.api('GET', '/api/v1/ops/incidents');
      const incidents = data.incidents || [];
      const incidentsEl = document.getElementById('card-incidents');
      if (incidentsEl) {
        incidentsEl.textContent = String(data.total_active || incidents.length || 0);
        incidentsEl.className = 'card-value' + ((data.total_active || incidents.length || 0) > 0 ? ' error' : '');
      }

      if (incidents.length === 0) {
        mnemo.setHtml('incident-panel', '<p class="muted">No active incidents. System is clear.</p>');
        return;
      }

      const severityTone = sev => sev === 'high' ? 'red' : (sev === 'medium' ? 'yellow' : 'blue');
      const rows = incidents.map(incident => {
        const traceHref = incident.request_id ? `/_/traces/${encodeURIComponent(incident.request_id)}` : '';
        const requestLink = incident.request_id
          ? `<a class="link" href="${traceHref}" data-request-id="${escapeHtml(incident.request_id)}">req ${escapeHtml(truncId(incident.request_id))}</a>`
          : '';
        return `<div class="incident-card severity-${escapeHtml(incident.severity || 'low')}" data-action-href="${escapeHtml(incident.action_href || '/_/')}" data-request-id="${escapeHtml(incident.request_id || '')}">
          <div class="incident-head">
            <div class="incident-title-wrap">
              ${badge(incident.kind.replaceAll('_', ' '), severityTone(incident.severity))}
              <strong>${escapeHtml(incident.title)}</strong>
            </div>
            <button class="btn btn-sm">${escapeHtml(incident.action_label || 'Open')}</button>
          </div>
          <p class="incident-summary">${escapeHtml(incident.summary || '')}</p>
          <div class="incident-meta">
            ${incident.resource_label ? `<span>${escapeHtml(incident.resource_label)}</span>` : ''}
            ${incident.opened_at ? `<span>${fmtDateAgo(incident.opened_at)}</span>` : ''}
            ${requestLink}
          </div>
        </div>`;
      }).join('');
      mnemo.setHtml('incident-panel', rows);

      document.querySelectorAll('#incident-panel .incident-card').forEach(card => {
        card.addEventListener('click', e => {
          const reqId = card.dataset.requestId;
          if (reqId && e.target.closest('a[data-request-id]')) {
            document.getElementById('trace-request-id').value = reqId;
            _navigate('traces', true, `/_/traces/${encodeURIComponent(reqId)}`);
            setTimeout(() => document.getElementById('trace-lookup-btn')?.click(), 50);
            return;
          }
          const href = card.dataset.actionHref || '/_/';
          const page = dashboardPageFromPath(href);
          _navigate(page, true, href);
        });
      });
    } catch (e) {
      mnemo.error('incident-panel', 'Failed to load active incidents: ' + e.message);
    }
  }, 10000);

  mnemo.poll(async () => {
    try {
      const data = await mnemo.api('GET', '/api/v1/memory/webhooks');
      const hooks = data.data || [];
      if (hooks.length === 0) {
        mnemo.setHtml('webhook-status-panel', '<p class="muted">No webhooks registered. <a class="link" href="/_/webhooks" onclick="event.preventDefault();_navigate(\'webhooks\')">Go to Webhooks →</a></p>');
        mnemo.setHtml('circuit-state-panel', '<p class="muted">No webhooks.</p>');
        return;
      }

      // Fetch stats for circuit state (parallel, best-effort)
      const statsArr = await Promise.all(
        hooks.map(h => mnemo.api('GET', `/api/v1/memory/webhooks/${h.id}/stats`).catch(() => null))
      );

      // Webhook status table
      const rows = hooks.map((h, i) => {
        const s = statsArr[i] || {};
        const st = h.enabled ? badge('enabled', 'green') : badge('disabled', 'yellow');
        const circ = s.circuit_open ? badge('open','red') : badge('closed','green');
        return `<tr class="clickable-row" onclick="_navigate('webhooks')">
          <td><code>${escapeHtml(truncId(h.id))}</code></td>
          <td>${escapeHtml(h.target_url)}</td>
          <td>${st}</td>
          <td>${circ}</td>
          <td>${s.dead_letter_events > 0 ? `<span style="color:var(--yellow)">${s.dead_letter_events}</span>` : 0}</td>
        </tr>`;
      }).join('');
      mnemo.setHtml('webhook-status-panel', `
        <div class="detail-header" style="margin-bottom:8px">
          <span style="font-size:12px;color:var(--text-muted)">${hooks.length} webhook${hooks.length !== 1 ? 's' : ''}</span>
          <a class="link" href="/_/webhooks" onclick="event.preventDefault();_navigate('webhooks')">Manage →</a>
        </div>
        <div class="table-wrap"><table>
          <thead><tr><th>ID</th><th>Target</th><th>Status</th><th>Circuit</th><th>Dead</th></tr></thead>
          <tbody>${rows}</tbody>
        </table></div>`);

      // Circuit state panel
      const circuitRows = hooks.map((h, i) => {
        const s = statsArr[i] || {};
        const dot = s.circuit_open ? '○' : '●';
        const col = s.circuit_open ? 'var(--red)' : 'var(--green)';
        const state = s.circuit_open
          ? `open — until ${fmtDateAgo(s.circuit_open_until)}`
          : `closed (healthy)`;
        return `<div class="circuit-row">
          <span class="circuit-dot" style="color:${col}">${dot}</span>
          <span class="circuit-url">${escapeHtml(h.target_url)}</span>
          <span class="circuit-state" style="color:${col}">${state}</span>
        </div>`;
      }).join('');
      mnemo.setHtml('circuit-state-panel', circuitRows);
    } catch (e) {
      mnemo.setHtml('webhook-status-panel', '<p class="muted">Could not load webhooks.</p>');
    }
  }, 15000);

  // Make metric cards clickable as deep-links
  const cardActions = {
    'card-webhooks':     'webhooks',
     'card-dead-letter':  'webhooks',
     'card-violations':   'governance',
     'card-incidents':    'webhooks',
     'card-http-requests': null,
    'card-status':       null,
    'card-version':      null,
  };
  Object.entries(cardActions).forEach(([id, target]) => {
    if (!target) return;
    const card = document.getElementById(id)?.closest('.card');
    if (card) {
      card.style.cursor = 'pointer';
      card.title = `Go to ${target}`;
      card.addEventListener('click', () => _navigate(target));
    }
  });
}

function openWebhookDetail(whId, pushUrl) {
  _selectedWebhookId = whId;
  document.querySelectorAll('.clickable-row[data-wh-id]').forEach(r => {
    r.classList.toggle('selected-row', r.dataset.whId === whId);
  });
  if (pushUrl !== false) {
    const query = location.search || '';
    history.pushState({ page: 'webhooks' }, '', `/_/webhooks/${encodeURIComponent(whId)}${query}`);
  }
  loadWebhookDetail(whId);
}

function navigateToTrace(requestId) {
  if (!requestId) return;
  const input = document.getElementById('trace-request-id');
  if (input) input.value = requestId;
  _navigate('traces', true, traceHref(requestId));
  setTimeout(() => document.getElementById('trace-lookup-btn')?.click(), 50);
}

function bindTraceLinks(scope) {
  (scope || document).querySelectorAll('a[data-trace-link]').forEach(link => {
    link.addEventListener('click', e => {
      e.preventDefault();
      navigateToTrace(link.dataset.traceLink);
    });
  });
}

// ═══════════════════════════════════════════════════════════════════
// WEBHOOKS PAGE
// ═══════════════════════════════════════════════════════════════════
let webhooksData = [];
let _selectedWebhookId = null;

function initWebhooks() {
  loadWebhookGrid();
}

async function loadWebhookGrid() {
  mnemo.loading('webhooks-grid');
  try {
    const filterMode = dashboardQuery().get('filter') || '';
    const list = await mnemo.api('GET', '/api/v1/memory/webhooks');
    webhooksData = list.data || [];
    if (webhooksData.length === 0) {
      mnemo.setHtml('webhooks-grid', '<p class="muted">No webhooks registered.</p>');
      mnemo.hide('webhook-detail');
      return;
    }
    const statsPromises = webhooksData.map(h =>
      mnemo.api('GET', `/api/v1/memory/webhooks/${h.id}/stats`).catch(() => null)
    );
    const allStats = await Promise.all(statsPromises);

    const filteredHooks = webhooksData.filter((h, i) => {
      const s = allStats[i] || {};
      if (filterMode === 'dead-letter') return (s.dead_letter_events || 0) > 0;
      if (filterMode === 'backlog') return (s.pending_events || 0) > 0 || (s.dead_letter_events || 0) > 0;
      return true;
    });

    const rows = filteredHooks.map(h => {
      const i = webhooksData.findIndex(row => row.id === h.id);
      const s = allStats[i] || {};
      const circuit = s.circuit_open ? badge('OPEN', 'red') : badge('closed', 'green');
      const selected = h.id === _selectedWebhookId ? ' selected-row' : '';
      return `<tr class="clickable-row${selected}" data-wh-id="${escapeHtml(h.id)}">
        <td><code>${escapeHtml(truncId(h.id))}</code></td>
        <td>${escapeHtml(h.target_url)}</td>
        <td>${h.enabled ? badge('enabled', 'green') : badge('disabled', 'yellow')}</td>
        <td>${circuit}</td>
        <td>${s.pending_events || 0}</td>
        <td>${(s.dead_letter_events||0) > 0 ? `<span style="color:var(--yellow)">${s.dead_letter_events}</span>` : 0}</td>
        <td>${s.delivered_events || 0}</td>
        <td>${escapeHtml(h.user_identifier)}</td>
      </tr>`;
    }).join('');

    const filterBanner = filterMode
      ? `<div class="detail-header" style="margin-bottom:8px"><span class="muted">Filtered to ${escapeHtml(filterMode.replace('-', ' '))} incidents</span><a class="link" href="/_/webhooks" data-reset-webhook-filter="true">Clear filter</a></div>`
      : '';

    mnemo.setHtml('webhooks-grid', `${filterBanner}<div class="table-wrap"><table>
      <thead><tr><th>ID</th><th>Target</th><th>Status</th><th>Circuit</th><th>Pending</th><th>Dead</th><th>Delivered</th><th>User</th></tr></thead>
      <tbody>${rows}</tbody></table></div>`);

    document.querySelectorAll('[data-reset-webhook-filter]').forEach(link => {
      link.addEventListener('click', e => {
        e.preventDefault();
        history.pushState({ page: 'webhooks' }, '', '/_/webhooks');
        _navigate('webhooks', false);
      });
    });

    document.querySelectorAll('.clickable-row[data-wh-id]').forEach(row => {
      row.addEventListener('click', () => {
        openWebhookDetail(row.dataset.whId, true);
      });
    });

    // Auto-reload selected webhook detail
    if (_selectedWebhookId) openWebhookDetail(_selectedWebhookId, false);
    if (!_selectedWebhookId && filteredHooks.length === 1) openWebhookDetail(filteredHooks[0].id, false);
  } catch (e) {
    mnemo.error('webhooks-grid', 'Failed to load webhooks: ' + e.message);
  }
}

async function loadWebhookDetail(whId) {
  mnemo.show('webhook-detail');
  mnemo.loading('wh-detail-content');
  try {
    const query = dashboardQuery();
    const focus = query.get('focus') || '';
    const [wh, stats, deadLetters, audit] = await Promise.all([
      mnemo.api('GET', `/api/v1/memory/webhooks/${whId}`),
      mnemo.api('GET', `/api/v1/memory/webhooks/${whId}/stats`),
      mnemo.api('GET', `/api/v1/memory/webhooks/${whId}/events/dead-letter?limit=50`),
      mnemo.api('GET', `/api/v1/memory/webhooks/${whId}/audit?limit=50`),
    ]);

    const circuit = stats.circuit_open
      ? `${badge('OPEN', 'red')} <span class="muted" style="font-size:11px">until ${fmtDate(stats.circuit_open_until)}</span>`
      : badge('closed', 'green');

    const dlEvents = deadLetters.events || [];
    let dlHtml = '';
    if (dlEvents.length > 0) {
      const dlRows = dlEvents.map(ev => `<tr>
        <td><code>${escapeHtml(truncId(ev.id))}</code></td>
        <td>${escapeHtml(ev.event_type)}</td>
        <td>${ev.attempts}</td>
        <td style="font-size:11px">${escapeHtml(ev.last_error || '--')}</td>
        <td>${fmtDate(ev.created_at)}</td>
        <td><button class="btn btn-xs" onclick="retryEvent('${escapeHtml(whId)}','${escapeHtml(ev.id)}')">Retry</button></td>
      </tr>`).join('');
      dlHtml = `<div class="table-wrap"><table><thead><tr><th>ID</th><th>Type</th><th>Attempts</th><th>Error</th><th>Created</th><th>Action</th></tr></thead><tbody>${dlRows}</tbody></table></div>`;
    } else {
      dlHtml = '<p class="muted">No dead-letter events. Queue is clean.</p>';
    }

    const audits = audit.audit || [];
    let auditHtml = '';
    if (audits.length > 0) {
      const auditRows = audits.map(a => `<tr>
        <td>${escapeHtml(a.action)}</td>
        <td style="font-size:11px">${traceLink(a.request_id)}</td>
        <td style="font-size:11px">${escapeHtml(JSON.stringify(a.details).substring(0, 80))}</td>
        <td>${fmtDateAgo(a.at)}</td>
      </tr>`).join('');
      auditHtml = `<div class="table-wrap"><table><thead><tr><th>Action</th><th>Request ID</th><th>Details</th><th>Time</th></tr></thead><tbody>${auditRows}</tbody></table></div>`;
    } else {
      auditHtml = '<p class="muted">No audit records.</p>';
    }

    mnemo.setHtml('wh-detail-content', `
      <div class="detail-header">
        <h2>Webhook <code>${escapeHtml(truncId(whId))}</code></h2>
        <div class="btn-group">
          <button class="btn btn-sm" onclick="replayWebhook('${escapeHtml(whId)}')">Replay All</button>
          <button class="btn btn-sm btn-danger" onclick="deleteWebhook('${escapeHtml(whId)}')">Delete</button>
        </div>
      </div>
      ${focus === 'dead-letter' ? '<div class="panel" style="margin-bottom:16px"><strong>Incident focus:</strong> dead-letter recovery lane</div>' : ''}
      <div class="stat-grid" style="margin-bottom:16px">
        <div class="stat-row"><span>Target</span><span style="word-break:break-all;font-size:12px">${escapeHtml(wh.target_url)}</span></div>
        <div class="stat-row"><span>Status</span><span>${wh.enabled ? badge('enabled','green') : badge('disabled','yellow')}</span></div>
        <div class="stat-row"><span>Circuit</span><span>${circuit}</span></div>
        <div class="stat-row"><span>Total</span><span>${stats.total_events || 0}</span></div>
        <div class="stat-row"><span>Delivered</span><span>${stats.delivered_events || 0}</span></div>
        <div class="stat-row"><span>Pending</span><span>${stats.pending_events || 0}</span></div>
        <div class="stat-row"><span>Dead-Letter</span><span>${(stats.dead_letter_events||0) > 0 ? `<span style="color:var(--yellow)">${stats.dead_letter_events}</span>` : 0}</span></div>
      </div>

      <h2>Dead-Letter Queue</h2>
      <div class="panel">${dlHtml}</div>

      <h2>Audit Log</h2>
      <div class="panel">${auditHtml}</div>
    `);
    bindTraceLinks(document.getElementById('wh-detail-content'));
  } catch (e) {
    mnemo.error('wh-detail-content', 'Failed to load webhook detail: ' + e.message);
  }
}

async function retryEvent(whId, eventId) {
  try {
    const res = await mnemo.api('POST', `/api/v1/memory/webhooks/${whId}/events/${eventId}/retry`, {});
    if (res.queued) {
      toast.success('Retry queued', 'Event will be retried shortly.');
    } else {
      toast.warn('Not queued', res.reason || 'Unknown reason.');
    }
    loadWebhookDetail(whId);
  } catch (e) { toast.error('Retry failed', e.message); }
}

async function replayWebhook(whId) {
  if (!await confirmAction('Replay all events for this webhook?', 'Confirm Replay')) return;
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
    toast.success('Replay complete', `Scanned ${total} events.`);
    loadWebhookDetail(whId);
  } catch (e) { toast.error('Replay failed', e.message); }
}

async function deleteWebhook(whId) {
  if (!await confirmAction('Permanently delete this webhook? This cannot be undone.', 'Delete Webhook')) return;
  try {
    await mnemo.api('DELETE', `/api/v1/memory/webhooks/${whId}`);
    toast.success('Webhook deleted', `ID: ${truncId(whId)}`);
    _selectedWebhookId = null;
    const query = location.search || '';
    history.pushState({ page: 'webhooks' }, '', `/_/webhooks${query}`);
    mnemo.hide('webhook-detail');
    loadWebhookGrid();
  } catch (e) { toast.error('Delete failed', e.message); }
}

// ═══════════════════════════════════════════════════════════════════
// RCA PAGE
// ═══════════════════════════════════════════════════════════════════
function initRca() {
  // Default to/from: last 7 days
  const now = new Date();
  const week = new Date(now.getTime() - 7 * 24 * 60 * 60 * 1000);
  const toLocal = d => {
    const pad = n => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
  };
  const fromEl = document.getElementById('rca-from');
  const toEl   = document.getElementById('rca-to');
  if (fromEl && !fromEl.value) fromEl.value = toLocal(week);
  if (toEl   && !toEl.value)   toEl.value   = toLocal(now);

  document.getElementById('rca-form').addEventListener('submit', async e => {
    e.preventDefault();
    const user = document.getElementById('rca-user').value.trim();
    const query = document.getElementById('rca-query').value.trim();
    const from = toIso(document.getElementById('rca-from').value);
    const to = toIso(document.getElementById('rca-to').value);
    const contract = document.getElementById('rca-contract').value;
    const policy = document.getElementById('rca-policy').value;
    if (!user || !query || !from || !to) {
      toast.warn('Missing fields', 'User, query, from, and to are required.');
      return;
    }

    mnemo.loading('rca-results');
    mnemo.show('rca-results');
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

function buildTimelineSvg(events) {
  if (!events || events.length === 0) return '<p class="muted">No timeline events.</p>';

  const W = 900;
  const H = 160;
  const PAD = 48;
  const AXIS_Y = 80;
  const DOT_R = 6;

  const times = events.map(e => new Date(e.at).getTime());
  const tMin = Math.min(...times);
  const tMax = Math.max(...times);
  const tRange = tMax - tMin || 1;

  const typeColor = {
    write: 'var(--green)',
    read: 'var(--blue)',
    delete: 'var(--red)',
    policy: 'var(--yellow)',
  };

  const xOf = t => PAD + ((t - tMin) / tRange) * (W - PAD * 2);

  let dots = '';
  let labels = '';

  events.forEach((ev, i) => {
    const x = xOf(new Date(ev.at).getTime());
    const color = typeColor[ev.event_type?.split('_')[0]] || 'var(--text-muted)';
    const above = i % 2 === 0;
    const labelY = above ? AXIS_Y - DOT_R - 28 : AXIS_Y + DOT_R + 20;
    const timeY  = above ? AXIS_Y - DOT_R - 10 : AXIS_Y + DOT_R + 36;
    const anchor = 'middle';

    dots += `<circle cx="${x}" cy="${AXIS_Y}" r="${DOT_R}" fill="${color}" stroke="var(--border)" stroke-width="1.5"/>`;
    if (i > 0) {
      const px = xOf(times[i - 1]);
      dots += `<line x1="${px}" y1="${AXIS_Y}" x2="${x}" y2="${AXIS_Y}" stroke="var(--border)" stroke-width="1.5"/>`;
    }
    labels += `<text x="${x}" y="${labelY}" text-anchor="${anchor}" font-size="10" fill="var(--text-muted)"
      style="font-family:var(--font-mono)">${escapeHtml((ev.event_type||'').substring(0,16))}</text>`;
    labels += `<text x="${x}" y="${timeY}" text-anchor="${anchor}" font-size="9" fill="var(--text-dim)"
      style="font-family:var(--font-mono)">${fmtDateShort(ev.at)}</text>`;
  });

  // Axis line
  const axisLine = `<line x1="${PAD}" y1="${AXIS_Y}" x2="${W - PAD}" y2="${AXIS_Y}" stroke="var(--border)" stroke-width="2"/>`;
  // Start/end ticks
  const ticks = `
    <line x1="${PAD}" y1="${AXIS_Y - 8}" x2="${PAD}" y2="${AXIS_Y + 8}" stroke="var(--border)" stroke-width="1.5"/>
    <line x1="${W - PAD}" y1="${AXIS_Y - 8}" x2="${W - PAD}" y2="${AXIS_Y + 8}" stroke="var(--border)" stroke-width="1.5"/>
    <text x="${PAD}" y="${AXIS_Y + 24}" text-anchor="middle" font-size="9" fill="var(--text-dim)"
      style="font-family:var(--font-mono)">${fmtDateShort(events[0]?.at)}</text>
    <text x="${W - PAD}" y="${AXIS_Y + 24}" text-anchor="middle" font-size="9" fill="var(--text-dim)"
      style="font-family:var(--font-mono)">${fmtDateShort(events[events.length-1]?.at)}</text>`;

  return `<div class="timeline-svg-wrap">
    <svg class="timeline-svg" viewBox="0 0 ${W} ${H}" xmlns="http://www.w3.org/2000/svg">
      ${axisLine}${ticks}${dots}${labels}
    </svg>
  </div>`;
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
      <div class="card"><div class="card-label">FROM — ${fmtDateShort(sf.as_of)}</div>
        <div class="stat-grid">
          <div class="stat-row"><span>Facts</span><span>${sf.fact_count || 0}</span></div>
          <div class="stat-row"><span>Episodes</span><span>${sf.episode_count || 0}</span></div>
          <div class="stat-row"><span>Tokens</span><span>${sf.token_count || 0}</span></div>
        </div>
      </div>
      <div class="card"><div class="card-label">TO — ${fmtDateShort(st.as_of)}</div>
        <div class="stat-grid">
          <div class="stat-row"><span>Facts</span><span>${st.fact_count || 0}</span></div>
          <div class="stat-row"><span>Episodes</span><span>${st.episode_count || 0}</span></div>
          <div class="stat-row"><span>Tokens</span><span>${st.token_count || 0}</span></div>
        </div>
      </div>
    </div>

    <h2>Gained Facts (${gained.length})</h2>
    <div class="panel">${gained.length > 0
      ? '<ul>' + gained.map(f => `<li>${escapeHtml(f.fact || f.text || JSON.stringify(f))}</li>`).join('') + '</ul>'
      : '<p class="muted">No facts gained in this window.</p>'}</div>

    <h2>Lost Facts (${lost.length})</h2>
    <div class="panel">${lost.length > 0
      ? '<ul>' + lost.map(f => `<li style="color:var(--red)">${escapeHtml(f.fact || f.text || JSON.stringify(f))}</li>`).join('') + '</ul>'
      : '<p class="muted">No facts lost in this window.</p>'}</div>

    <h2>Timeline (${timeline.length} events)</h2>
    ${buildTimelineSvg(timeline)}

    <h2>Retrieval Policy Diagnostics</h2>
    <div class="card-grid">
      <div class="card"><div class="card-label">Max Tokens</div><div class="card-value">${diag.effective_max_tokens || '--'}</div></div>
      <div class="card"><div class="card-label">Min Relevance</div><div class="card-value">${diag.effective_min_relevance != null ? diag.effective_min_relevance.toFixed(2) : '--'}</div></div>
      <div class="card"><div class="card-label">Temporal Intent</div><div class="card-value" style="font-size:16px">${escapeHtml(diag.effective_temporal_intent || '--')}</div></div>
      <div class="card"><div class="card-label">Temporal Weight</div><div class="card-value">${diag.effective_temporal_weight != null ? diag.effective_temporal_weight.toFixed(2) : '--'}</div></div>
    </div>

    <h2>Summary</h2>
    <div class="panel"><p>${escapeHtml(d.summary || '--')}</p></div>
    <p class="muted" style="font-size:11px">Contract: ${escapeHtml(d.contract_applied)} &middot; Policy: ${escapeHtml(d.retrieval_policy_applied)}</p>
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
    if (!user) { toast.warn('Input required', 'Enter a username.'); return; }
    govCurrentUser = user;
    history.pushState({ page: 'governance' }, '', `/_/governance/${encodeURIComponent(user)}`);
    loadGovernance(user);
  });
  document.getElementById('gov-user').addEventListener('keydown', e => {
    if (e.key === 'Enter') document.getElementById('gov-load-btn').click();
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
        <h2>Policy — ${escapeHtml(p.user_identifier)}</h2>
        <div class="btn-group">
          <button class="btn btn-sm" id="gov-edit-btn">Edit</button>
          <button class="btn btn-sm" id="gov-preview-btn">Preview Impact</button>
        </div>
      </div>
      <div class="stat-grid">
        <div class="stat-row"><span>Retention (messages)</span><span>${p.retention_days_message} days</span></div>
        <div class="stat-row"><span>Retention (text)</span><span>${p.retention_days_text} days</span></div>
        <div class="stat-row"><span>Retention (JSON)</span><span>${p.retention_days_json} days</span></div>
        <div class="stat-row"><span>Domain Allowlist</span><span>${escapeHtml((p.webhook_domain_allowlist||[]).join(', ') || '(any)')}</span></div>
        <div class="stat-row"><span>Default Contract</span><span>${badge(p.default_memory_contract, 'blue')}</span></div>
        <div class="stat-row"><span>Default Policy</span><span>${badge(p.default_retrieval_policy, 'blue')}</span></div>
        <div class="stat-row"><span>Updated</span><span>${fmtDateAgo(p.updated_at)}</span></div>
      </div>
      <div id="gov-edit-form" class="hidden" style="margin-top:16px">
        <div class="form-row">
          <label>Retention Messages (days)<input type="number" id="gov-ret-msg" value="${p.retention_days_message}" autocomplete="off"></label>
          <label>Retention Text (days)<input type="number" id="gov-ret-txt" value="${p.retention_days_text}" autocomplete="off"></label>
          <label>Retention JSON (days)<input type="number" id="gov-ret-json" value="${p.retention_days_json}" autocomplete="off"></label>
        </div>
        <div class="form-row">
          <label>Domain Allowlist (comma-separated)<input type="text" id="gov-allowlist" value="${escapeHtml((p.webhook_domain_allowlist||[]).join(', '))}" autocomplete="off"></label>
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
          <button type="button" class="btn btn-success" id="gov-save-btn">Save</button>
        </div>
      </div>
    `);
    mnemo.show('gov-policy-panel');

    document.getElementById('gov-edit-btn').addEventListener('click', () => {
      document.getElementById('gov-edit-form').classList.toggle('hidden');
    });
    document.getElementById('gov-save-btn').addEventListener('click', () => saveGovernance(user));
    document.getElementById('gov-preview-btn').addEventListener('click', () => previewGovernance(user));

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
    toast.success('Policy saved', `Updated for ${user}`);
    loadGovernance(user);
  } catch (e) { toast.error('Save failed', e.message); }
}

async function previewGovernance(user) {
  const editForm = document.getElementById('gov-edit-form');
  if (editForm && editForm.classList.contains('hidden')) {
    editForm.classList.remove('hidden');
    toast.info('Edit fields first', 'Update the form values, then click Preview Impact.');
    return;
  }
  try {
    const res = await mnemo.api('POST', `/api/v1/policies/${encodeURIComponent(user)}/preview`, getGovFormData());
    toast.info('Preview Impact',
      `~${res.estimated_affected_episodes_total} episodes affected (${res.confidence}) — ` +
      `Msg: ${res.estimated_affected_message_episodes}, Text: ${res.estimated_affected_text_episodes}, JSON: ${res.estimated_affected_json_episodes}`);
  } catch (e) { toast.error('Preview failed', e.message); }
}

async function loadViolations(user) {
  mnemo.loading('gov-violations-panel');
  try {
    const now = new Date();
    const from = new Date(now.getTime() - 24 * 60 * 60 * 1000).toISOString();
    const data = await mnemo.api('GET', `/api/v1/policies/${encodeURIComponent(user)}/violations?from=${from}&to=${now.toISOString()}&limit=50`);
    const viols = data.violations || [];
    if (viols.length === 0) {
      mnemo.setHtml('gov-violations-panel', '<h2>Violations (24h)</h2><p class="muted">None. All clear.</p>');
    } else {
      const rows = viols.map(v => `<tr>
        <td>${escapeHtml(v.action)}</td>
        <td style="font-size:11px">${traceLink(v.request_id)}</td>
        <td>${fmtDateAgo(v.at)}</td>
      </tr>`).join('');
      mnemo.setHtml('gov-violations-panel', `<h2>Violations (24h) — ${badge(viols.length,'red')}</h2>
        <div class="panel"><div class="table-wrap"><table><thead><tr><th>Action</th><th>Request ID</th><th>Time</th></tr></thead><tbody>${rows}</tbody></table></div></div>`);
    }
    mnemo.show('gov-violations-panel');
    bindTraceLinks(document.getElementById('gov-violations-panel'));
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
        <td style="font-size:11px">${traceLink(a.request_id)}</td>
        <td style="font-size:11px">${escapeHtml(JSON.stringify(a.details).substring(0, 100))}</td>
        <td>${fmtDateAgo(a.at)}</td>
      </tr>`).join('');
      mnemo.setHtml('gov-audit-panel', `<h2>Audit Trail — ${audits.length}</h2>
        <div class="panel"><div class="table-wrap"><table><thead><tr><th>Action</th><th>Request ID</th><th>Details</th><th>Time</th></tr></thead><tbody>${rows}</tbody></table></div></div>`);
    }
    mnemo.show('gov-audit-panel');
    bindTraceLinks(document.getElementById('gov-audit-panel'));
  } catch (e) {
    mnemo.error('gov-audit-panel', 'Could not load audit trail.');
  }
}

// ═══════════════════════════════════════════════════════════════════
// TRACES PAGE
// ═══════════════════════════════════════════════════════════════════
function initTraces() {
  const btn   = document.getElementById('trace-lookup-btn');
  const input = document.getElementById('trace-request-id');

  // Set default window: last 30 days
  const now  = new Date();
  const ago30 = new Date(now.getTime() - 30 * 24 * 60 * 60 * 1000);
  const toLocal = d => {
    const pad = n => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
  };
  const trFrom = document.getElementById('trace-from');
  const trTo   = document.getElementById('trace-to');
  if (trFrom && !trFrom.value) trFrom.value = toLocal(ago30);
  if (trTo   && !trTo.value)   trTo.value   = toLocal(now);

  async function doLookup() {
    const reqId = input.value.trim();
    if (!reqId) { toast.warn('Input required', 'Enter a request ID.'); return; }
    mnemo.loading('trace-results');
    mnemo.show('trace-results');
    try {
      // Build query params from source filter checkboxes + time window
      const params = new URLSearchParams();
      const fromVal = trFrom && trFrom.value ? toIso(trFrom.value) : null;
      const toVal   = trTo   && trTo.value   ? toIso(trTo.value)   : null;
      if (fromVal) params.set('from', fromVal);
      if (toVal)   params.set('to', toVal);
      const chkEps  = document.getElementById('trace-chk-episodes');
      const chkWh   = document.getElementById('trace-chk-webhooks');
      const chkGov  = document.getElementById('trace-chk-governance');
      if (chkEps  && !chkEps.checked)  params.set('include_episodes', 'false');
      if (chkWh   && !chkWh.checked)   params.set('include_webhook_events', 'false');
      if (chkGov  && !chkGov.checked)  params.set('include_governance_audit', 'false');
      const qs = params.toString();
      const data = await mnemo.api('GET', `/api/v1/traces/${encodeURIComponent(reqId)}${qs ? '?' + qs : ''}`);
      history.pushState({ page: 'traces' }, '', `/_/traces/${encodeURIComponent(reqId)}`);
      renderTraceResults(data);
    } catch (e) {
      mnemo.error('trace-results', 'Trace lookup failed: ' + e.message);
    }
  }

  btn.addEventListener('click', doLookup);
  input.addEventListener('keydown', e => { if (e.key === 'Enter') doLookup(); });
}

function renderTraceResults(d) {
  const eps = d.matched_episodes || [];
  const whEvts = d.matched_webhook_events || [];
  const whAudit = d.matched_webhook_audit || [];
  const govAudit = d.matched_governance_audit || [];

  let html = `<h2>Trace: <code>${escapeHtml(d.request_id)}</code></h2>
    <div class="card-grid" style="margin-bottom:16px">
      <div class="card"><div class="card-label">Episodes</div><div class="card-value">${eps.length}</div></div>
      <div class="card"><div class="card-label">Webhook Events</div><div class="card-value">${whEvts.length}</div></div>
      <div class="card"><div class="card-label">Webhook Audit</div><div class="card-value">${whAudit.length}</div></div>
      <div class="card"><div class="card-label">Governance Audit</div><div class="card-value">${govAudit.length}</div></div>
    </div>`;

  if (eps.length > 0) {
    const rows = eps.map(e => `<tr>
      <td><code>${escapeHtml(truncId(e.episode_id))}</code></td>
      <td><code>${escapeHtml(truncId(e.session_id))}</code></td>
      <td>${escapeHtml(e.preview)}</td>
      <td>${fmtDateAgo(e.created_at)}</td>
    </tr>`).join('');
    html += `<h2>Episodes</h2><div class="panel"><div class="table-wrap"><table>
      <thead><tr><th>Episode ID</th><th>Session</th><th>Preview</th><th>Created</th></tr></thead>
      <tbody>${rows}</tbody></table></div></div>`;
  }

  if (whEvts.length > 0) {
    const rows = whEvts.map(e => `<tr>
      <td><code>${escapeHtml(truncId(e.id))}</code></td>
      <td>${escapeHtml(e.event_type)}</td>
      <td>${e.delivered ? badge('yes','green') : badge('no','gray')}</td>
      <td>${e.dead_letter ? badge('yes','red') : badge('no','gray')}</td>
      <td>${fmtDateAgo(e.created_at)}</td>
    </tr>`).join('');
    html += `<h2>Webhook Events</h2><div class="panel"><div class="table-wrap"><table>
      <thead><tr><th>ID</th><th>Type</th><th>Delivered</th><th>Dead Letter</th><th>Created</th></tr></thead>
      <tbody>${rows}</tbody></table></div></div>`;
  }

  if (whAudit.length > 0) {
    const rows = whAudit.map(a => `<tr>
      <td>${escapeHtml(a.action)}</td>
      <td style="font-size:11px">${escapeHtml(JSON.stringify(a.details).substring(0,100))}</td>
      <td>${fmtDateAgo(a.at)}</td>
    </tr>`).join('');
    html += `<h2>Webhook Audit</h2><div class="panel"><div class="table-wrap"><table>
      <thead><tr><th>Action</th><th>Details</th><th>Time</th></tr></thead>
      <tbody>${rows}</tbody></table></div></div>`;
  }

  if (govAudit.length > 0) {
    const rows = govAudit.map(a => `<tr>
      <td>${escapeHtml(a.action)}</td>
      <td style="font-size:11px">${escapeHtml(JSON.stringify(a.details).substring(0,100))}</td>
      <td>${fmtDateAgo(a.at)}</td>
    </tr>`).join('');
    html += `<h2>Governance Audit</h2><div class="panel"><div class="table-wrap"><table>
      <thead><tr><th>Action</th><th>Details</th><th>Time</th></tr></thead>
      <tbody>${rows}</tbody></table></div></div>`;
  }

  if (eps.length === 0 && whEvts.length === 0 && whAudit.length === 0 && govAudit.length === 0) {
    html += '<p class="muted">No records found for this request ID.</p>';
  }

  mnemo.setHtml('trace-results', html);
  mnemo.show('trace-results');
}

// ═══════════════════════════════════════════════════════════════════
// EXPLORER PAGE — D3 v7 force-directed graph
// ═══════════════════════════════════════════════════════════════════
let _graphSim = null;
let _graphNodes = [];
let _graphEdges = [];
let _graphTransform = { x: 0, y: 0, k: 1 };
let _selectedNode = null;
let _dragNode = null;
let _dragOffX = 0;
let _dragOffY = 0;
let _isPanning = false;
let _panStartX = 0;
let _panStartY = 0;
let _panOriginX = 0;
let _panOriginY = 0;
let _graphW = 0;
let _graphH = 0;
let _rafId = null;

const NODE_COLORS = {
  person: '#6366f1',
  location: '#22c55e',
  organization: '#eab308',
  concept: '#818cf8',
  event: '#f59e0b',
  object: '#06b6d4',
};

function nodeColor(type) {
  return NODE_COLORS[(type||'').toLowerCase()] || '#9ca3af';
}

function initExplorer() {
  document.getElementById('explorer-load-btn').addEventListener('click', () => loadEntities());
  document.getElementById('explorer-user').addEventListener('keydown', e => {
    if (e.key === 'Enter') loadEntities();
  });
  const resetBtn = document.getElementById('explorer-reset-btn');
  if (resetBtn) resetBtn.addEventListener('click', resetGraphView);

  const zoomIn = document.getElementById('graph-zoom-in');
  const zoomOut = document.getElementById('graph-zoom-out');
  const fit = document.getElementById('graph-fit');
  if (zoomIn) zoomIn.addEventListener('click', () => zoomGraph(1.3));
  if (zoomOut) zoomOut.addEventListener('click', () => zoomGraph(0.77));
  if (fit) fit.addEventListener('click', fitGraph);
}

async function loadEntities() {
  const user = document.getElementById('explorer-user').value.trim();
  if (!user) { toast.warn('Input required', 'Enter a username.'); return; }
  mnemo.loading('explorer-entities');
  mnemo.hide('explorer-graph');
  mnemo.hide('node-detail-panel');

  try {
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
      <td>${badge(e.entity_type || 'unknown', 'blue')}</td>
      <td style="font-size:11px">${escapeHtml(e.summary || '--')}</td>
    </tr>`).join('');

    mnemo.setHtml('explorer-entities', `
      <h2>Entities (${entities.length})</h2>
      <div class="table-wrap"><table>
        <thead><tr><th>ID</th><th>Name</th><th>Type</th><th>Summary</th></tr></thead>
        <tbody>${rows}</tbody>
      </table></div>
      <p class="muted" style="margin-top:8px;font-size:12px">Click an entity to load its subgraph.</p>`);
    mnemo.show('explorer-entities');

    document.querySelectorAll('.clickable-row[data-entity-id]').forEach(row => {
      row.addEventListener('click', () => loadSubgraph(row.dataset.entityId));
    });
  } catch (e) {
    mnemo.error('explorer-entities', 'Failed to load entities: ' + e.message);
  }
}

async function loadSubgraph(entityId) {
  mnemo.show('explorer-graph');
  try {
    const graphData = await mnemo.api('GET', `/api/v1/entities/${entityId}/subgraph?depth=2&max_nodes=50`);
    renderD3Graph(graphData, entityId);
  } catch (e) {
    const canvas = document.getElementById('graph-canvas');
    if (canvas) {
      const ctx = canvas.getContext('2d');
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.fillStyle = '#6b7280';
      ctx.font = '14px monospace';
      ctx.textAlign = 'center';
      ctx.fillText('Failed to load subgraph: ' + e.message, canvas.width / 2, canvas.height / 2);
    }
  }
}

function renderD3Graph(data, seedId) {
  const canvas = document.getElementById('graph-canvas');
  if (!canvas) return;

  // CSS-sized canvas: clientWidth/Height are set by the .graph-container rule (520px)
  _graphW = canvas.width  = canvas.offsetWidth  || 800;
  _graphH = canvas.height = canvas.offsetHeight || 520;
  _graphTransform = { x: 0, y: 0, k: 1 };
  _selectedNode = null;
  mnemo.hide('node-detail-panel');

  _graphNodes = (data.nodes || []).map(n => ({
    id: n.entity.id,
    label: n.entity.name || truncId(n.entity.id),
    type: (n.entity.entity_type || 'unknown').toLowerCase(),
    depth: n.depth,
    summary: n.entity.summary || '',
    x: _graphW / 2 + (Math.random() - 0.5) * 200,
    y: _graphH / 2 + (Math.random() - 0.5) * 200,
    vx: 0, vy: 0,
  }));

  const nodeById = {};
  _graphNodes.forEach(n => nodeById[n.id] = n);

  _graphEdges = (data.edges || [])
    .filter(e => nodeById[e.source_entity_id] && nodeById[e.target_entity_id])
    .map(e => ({
      source: nodeById[e.source_entity_id],
      target: nodeById[e.target_entity_id],
      label: e.label || '',
      invalid: !!e.invalid_at,
    }));

  // Build legend
  buildGraphLegend();

  if (typeof d3 !== 'undefined') {
    runD3Simulation(seedId);
  } else {
    runFallbackSimulation(seedId);
  }

  setupCanvasInteractions(canvas, seedId);
}

function runD3Simulation(seedId) {
  if (_graphSim) _graphSim.stop();

  _graphSim = d3.forceSimulation(_graphNodes)
    .force('link', d3.forceLink(_graphEdges).id(n => n.id).distance(100).strength(0.5))
    .force('charge', d3.forceManyBody().strength(-300))
    .force('center', d3.forceCenter(_graphW / 2, _graphH / 2))
    .force('collide', d3.forceCollide(20))
    .alphaDecay(0.02)
    .on('tick', () => drawGraph(document.getElementById('graph-canvas')));
}

function runFallbackSimulation(seedId) {
  const canvas = document.getElementById('graph-canvas');
  const ITERATIONS = 150;
  const REPULSION = 3500;
  const SPRING = 0.01;
  const REST = 110;
  const DAMPING = 0.85;

  for (let iter = 0; iter < ITERATIONS; iter++) {
    for (let i = 0; i < _graphNodes.length; i++) {
      for (let j = i + 1; j < _graphNodes.length; j++) {
        const dx = _graphNodes[j].x - _graphNodes[i].x;
        const dy = _graphNodes[j].y - _graphNodes[i].y;
        const dist = Math.sqrt(dx * dx + dy * dy) || 1;
        const force = REPULSION / (dist * dist);
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        _graphNodes[i].vx -= fx; _graphNodes[i].vy -= fy;
        _graphNodes[j].vx += fx; _graphNodes[j].vy += fy;
      }
    }
    for (const e of _graphEdges) {
      const dx = e.target.x - e.source.x;
      const dy = e.target.y - e.source.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 1;
      const force = SPRING * (dist - REST);
      const fx = (dx / dist) * force;
      const fy = (dy / dist) * force;
      e.source.vx += fx; e.source.vy += fy;
      e.target.vx -= fx; e.target.vy -= fy;
    }
    for (const n of _graphNodes) {
      n.vx *= DAMPING; n.vy *= DAMPING;
      n.x += n.vx; n.y += n.vy;
      n.x = Math.max(20, Math.min(_graphW - 20, n.x));
      n.y = Math.max(20, Math.min(_graphH - 20, n.y));
    }
  }
  drawGraph(canvas);
}

function drawGraph(canvas) {
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  const W = canvas.width;
  const H = canvas.height;
  ctx.clearRect(0, 0, W, H);

  if (_graphNodes.length === 0) {
    ctx.fillStyle = '#6b7280';
    ctx.font = '14px monospace';
    ctx.textAlign = 'center';
    ctx.fillText('No nodes in subgraph.', W / 2, H / 2);
    return;
  }

  ctx.save();
  ctx.translate(_graphTransform.x, _graphTransform.y);
  ctx.scale(_graphTransform.k, _graphTransform.k);

  const zoom = _graphTransform.k;
  const labelFontSize = Math.max(8, Math.min(13, 11 / zoom));

  // Edges
  for (const e of _graphEdges) {
    const sx = e.source.x, sy = e.source.y;
    const tx = e.target.x, ty = e.target.y;
    ctx.beginPath();
    ctx.moveTo(sx, sy);
    ctx.lineTo(tx, ty);
    if (e.invalid) {
      ctx.setLineDash([4, 4]);
      ctx.strokeStyle = 'rgba(239,68,68,0.35)';
    } else {
      ctx.setLineDash([]);
      ctx.strokeStyle = 'rgba(99,102,241,0.35)';
    }
    ctx.lineWidth = 1.5;
    ctx.stroke();
    ctx.setLineDash([]);

    if (e.label && zoom > 0.6) {
      const mx = (sx + tx) / 2;
      const my = (sy + ty) / 2;
      ctx.fillStyle = '#6b7280';
      ctx.font = `${Math.max(7, 9 / zoom)}px monospace`;
      ctx.textAlign = 'center';
      ctx.fillText(e.label.substring(0, 20), mx, my - 4);
    }
  }

  // Nodes
  for (const n of _graphNodes) {
    const isSeed = n.depth === 0;
    const isSelected = n === _selectedNode;
    const r = isSeed ? 12 : 8;
    const color = nodeColor(n.type);

    // Glow for seed or selected
    if (isSeed || isSelected) {
      ctx.save();
      ctx.shadowColor = isSelected ? '#818cf8' : color;
      ctx.shadowBlur = 14;
    }

    ctx.beginPath();
    ctx.arc(n.x, n.y, r, 0, Math.PI * 2);
    ctx.fillStyle = color;
    ctx.fill();
    ctx.strokeStyle = isSelected ? '#818cf8' : (isSeed ? '#e1e4ed' : 'rgba(225,228,237,0.5)');
    ctx.lineWidth = isSelected ? 2.5 : (isSeed ? 2 : 1);
    ctx.stroke();

    if (isSeed || isSelected) ctx.restore();

    // Label
    ctx.fillStyle = '#d1d5db';
    ctx.font = (isSeed ? 'bold ' : '') + `${labelFontSize}px monospace`;
    ctx.textAlign = 'center';
    ctx.fillText(n.label.substring(0, 18), n.x, n.y + r + labelFontSize + 2);
  }

  ctx.restore();

  // Legend text bottom-left (screen space, not transformed)
  ctx.fillStyle = '#4b5563';
  ctx.font = '10px monospace';
  ctx.textAlign = 'left';
  ctx.fillText(`${_graphNodes.length} nodes · ${_graphEdges.length} edges`, 8, H - 8);
}

function buildGraphLegend() {
  const legend = document.getElementById('graph-legend');
  if (!legend) return;
  const types = [...new Set(_graphNodes.map(n => n.type))];
  if (types.length === 0) { legend.innerHTML = ''; return; }
  legend.innerHTML = types.map(t =>
    `<span class="legend-item"><span class="legend-dot" style="background:${nodeColor(t)}"></span>${escapeHtml(t)}</span>`
  ).join('');
}

function setupCanvasInteractions(canvas, seedId) {
  // Remove old listeners by replacing the canvas (easiest way with anon fns)
  const newCanvas = canvas.cloneNode(true);
  canvas.parentNode.replaceChild(newCanvas, canvas);
  const c = newCanvas;

  function canvasPos(e) {
    const rect = c.getBoundingClientRect();
    return {
      cx: (e.clientX - rect.left - _graphTransform.x) / _graphTransform.k,
      cy: (e.clientY - rect.top - _graphTransform.y) / _graphTransform.k,
      clientX: e.clientX,
      clientY: e.clientY,
      rectLeft: rect.left,
      rectTop: rect.top,
    };
  }

  function hitNode(cx, cy) {
    const HIT_R = 16;
    return _graphNodes.find(n => {
      const dx = n.x - cx, dy = n.y - cy;
      return dx * dx + dy * dy <= HIT_R * HIT_R;
    });
  }

  c.addEventListener('mousedown', e => {
    const { cx, cy } = canvasPos(e);
    const hit = hitNode(cx, cy);
    if (hit) {
      _dragNode = hit;
      _dragOffX = cx - hit.x;
      _dragOffY = cy - hit.y;
      if (_graphSim) _graphSim.alphaTarget(0.3).restart();
    } else {
      _isPanning = true;
      _panStartX = e.clientX;
      _panStartY = e.clientY;
      _panOriginX = _graphTransform.x;
      _panOriginY = _graphTransform.y;
    }
  });

  c.addEventListener('mousemove', e => {
    if (_dragNode) {
      const { cx, cy } = canvasPos(e);
      _dragNode.x = cx - _dragOffX;
      _dragNode.y = cy - _dragOffY;
      _dragNode.fx = _dragNode.x;
      _dragNode.fy = _dragNode.y;
      drawGraph(c);
    } else if (_isPanning) {
      _graphTransform.x = _panOriginX + (e.clientX - _panStartX);
      _graphTransform.y = _panOriginY + (e.clientY - _panStartY);
      drawGraph(c);
    } else {
      // Hover tooltip
      const { cx, cy, clientX, clientY, rectLeft, rectTop } = canvasPos(e);
      const hit = hitNode(cx, cy);
      showGraphTooltip(hit, clientX - rectLeft, clientY - rectTop);
    }
  });

  c.addEventListener('mouseup', e => {
    if (_dragNode) {
      _dragNode.fx = null;
      _dragNode.fy = null;
      if (_graphSim) _graphSim.alphaTarget(0);
    }
    _dragNode = null;
    _isPanning = false;
  });

  c.addEventListener('mouseleave', () => {
    _dragNode = null;
    _isPanning = false;
    showGraphTooltip(null, 0, 0);
  });

  c.addEventListener('click', e => {
    const { cx, cy } = canvasPos(e);
    const hit = hitNode(cx, cy);
    if (hit) {
      _selectedNode = hit;
      renderNodeDetail(hit);
      mnemo.show('node-detail-panel');
    } else {
      _selectedNode = null;
      mnemo.hide('node-detail-panel');
    }
    drawGraph(c);
  });

  c.addEventListener('wheel', e => {
    e.preventDefault();
    const rect = c.getBoundingClientRect();
    const mouseX = e.clientX - rect.left;
    const mouseY = e.clientY - rect.top;
    const factor = e.deltaY < 0 ? 1.15 : 0.87;
    const newK = Math.max(0.2, Math.min(4, _graphTransform.k * factor));
    _graphTransform.x = mouseX - (mouseX - _graphTransform.x) * (newK / _graphTransform.k);
    _graphTransform.y = mouseY - (mouseY - _graphTransform.y) * (newK / _graphTransform.k);
    _graphTransform.k = newK;
    drawGraph(c);
  }, { passive: false });
}

function showGraphTooltip(node, x, y) {
  const tip = document.getElementById('graph-tooltip');
  if (!tip) return;
  if (!node) { tip.classList.add('hidden'); return; }
  tip.innerHTML = `<strong>${escapeHtml(node.label)}</strong><br>
    <span class="muted">${escapeHtml(node.type)}</span>
    ${node.summary ? `<br><span style="font-size:11px">${escapeHtml(node.summary.substring(0,80))}</span>` : ''}`;
  tip.style.left = (x + 14) + 'px';
  tip.style.top = (y - 10) + 'px';
  tip.classList.remove('hidden');
}

function renderNodeDetail(node) {
  const panel = document.getElementById('node-detail-panel');
  if (!panel) return;
  panel.innerHTML = `
    <button class="btn btn-xs btn-ghost" onclick="document.getElementById('node-detail-panel').classList.add('hidden')" style="float:right">&times;</button>
    <div style="font-weight:600;font-size:14px;margin-bottom:4px">${escapeHtml(node.label)}</div>
    <div>${badge(node.type, 'blue')}</div>
    ${node.summary ? `<p style="font-size:12px;margin-top:8px;color:var(--text-muted)">${escapeHtml(node.summary)}</p>` : ''}
    <div class="stat-grid" style="margin-top:8px">
      <div class="stat-row"><span>ID</span><span><code>${escapeHtml(truncId(node.id))}</code></span></div>
      <div class="stat-row"><span>Depth</span><span>${node.depth}</span></div>
    </div>`;
}

function zoomGraph(factor) {
  const canvas = document.getElementById('graph-canvas');
  if (!canvas) return;
  const cx = canvas.width / 2;
  const cy = canvas.height / 2;
  const newK = Math.max(0.2, Math.min(4, _graphTransform.k * factor));
  _graphTransform.x = cx - (cx - _graphTransform.x) * (newK / _graphTransform.k);
  _graphTransform.y = cy - (cy - _graphTransform.y) * (newK / _graphTransform.k);
  _graphTransform.k = newK;
  drawGraph(canvas);
}

function fitGraph() {
  const canvas = document.getElementById('graph-canvas');
  if (!canvas || _graphNodes.length === 0) return;
  const W = canvas.width;
  const H = canvas.height;
  const PAD = 40;
  const xs = _graphNodes.map(n => n.x);
  const ys = _graphNodes.map(n => n.y);
  const minX = Math.min(...xs), maxX = Math.max(...xs);
  const minY = Math.min(...ys), maxY = Math.max(...ys);
  const rangeX = maxX - minX || 1;
  const rangeY = maxY - minY || 1;
  const k = Math.min((W - PAD * 2) / rangeX, (H - PAD * 2) / rangeY, 3);
  _graphTransform.k = k;
  _graphTransform.x = (W - (minX + maxX) * k) / 2;
  _graphTransform.y = (H - (minY + maxY) * k) / 2;
  drawGraph(canvas);
}

function resetGraphView() {
  _graphTransform = { x: 0, y: 0, k: 1 };
  const canvas = document.getElementById('graph-canvas');
  if (canvas) drawGraph(canvas);
}

// ═══════════════════════════════════════════════════════════════════
// BOOT
// ═══════════════════════════════════════════════════════════════════
document.addEventListener('DOMContentLoaded', () => {
  // Register lazy inits
  _pageInits['webhooks'] = { init: initWebhooks };
  _pageInits['rca'] = { init: initRca };
  _pageInits['governance'] = { init: initGovernance };
  _pageInits['traces'] = { init: initTraces };
  _pageInits['explorer'] = { init: initExplorer };

  initNav();

  // Home always inits immediately
  initHome();
  _pageInits['home'] = { init: () => {}, _done: true };
});
