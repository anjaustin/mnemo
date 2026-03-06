/* Mnemo Dashboard — Phase A skeleton JS */
'use strict';

const mnemo = {
  // ─── API helper ──────────────────────────────────────────────────
  async api(method, path, body) {
    const opts = {
      method,
      headers: { 'Content-Type': 'application/json' },
    };
    if (body) opts.body = JSON.stringify(body);
    const res = await fetch(path, opts);
    if (!res.ok) {
      const err = await res.json().catch(() => ({ error: { message: res.statusText } }));
      throw new Error(err.error?.message || res.statusText);
    }
    const ct = res.headers.get('content-type') || '';
    if (ct.includes('application/json')) return res.json();
    return res.text();
  },

  // ─── Polling helper ──────────────────────────────────────────────
  _intervals: [],
  poll(fn, intervalMs) {
    fn(); // run immediately
    const id = setInterval(fn, intervalMs);
    this._intervals.push(id);
    return id;
  },

  // ─── Simple text setter ──────────────────────────────────────────
  setText(id, text) {
    const el = document.getElementById(id);
    if (el) el.textContent = text;
  },

  setHtml(id, html) {
    const el = document.getElementById(id);
    if (el) el.innerHTML = html;
  },

  show(id) {
    const el = document.getElementById(id);
    if (el) el.classList.remove('hidden');
  },

  hide(id) {
    const el = document.getElementById(id);
    if (el) el.classList.add('hidden');
  },
};

// ─── Navigation ────────────────────────────────────────────────────
function initNav() {
  const links = document.querySelectorAll('.nav-link');
  const pages = document.querySelectorAll('.page');

  function navigate(pageName) {
    pages.forEach(p => p.classList.add('hidden'));
    links.forEach(l => l.classList.remove('active'));
    const page = document.getElementById('page-' + pageName);
    const link = document.querySelector(`[data-page="${pageName}"]`);
    if (page) page.classList.remove('hidden');
    if (link) link.classList.add('active');
  }

  links.forEach(link => {
    link.addEventListener('click', e => {
      e.preventDefault();
      const page = link.dataset.page;
      navigate(page);
      history.pushState({ page }, '', link.href);
    });
  });

  // Handle browser back/forward buttons
  window.addEventListener('popstate', () => {
    const page = location.pathname.replace(/^\/_\/?/, '').split('/')[0] || 'home';
    navigate(page);
  });

  // Handle initial route
  const path = location.pathname.replace(/^\/_\/?/, '').split('/')[0] || 'home';
  navigate(path);
}

// ─── Home page ─────────────────────────────────────────────────────
function initHome() {
  // Health polling (every 5s)
  mnemo.poll(async () => {
    try {
      const data = await mnemo.api('GET', '/health');
      mnemo.setText('card-status', data.status || 'ok');
      mnemo.setText('card-version', data.version || '--');
      mnemo.setText('nav-version', 'v' + (data.version || ''));
      mnemo.setText('health-label', data.status || 'ok');
      const dot = document.getElementById('health-dot');
      if (dot) {
        dot.className = 'health-dot ' + (data.status === 'ok' ? 'ok' : 'error');
      }
      const statusCard = document.getElementById('card-status');
      if (statusCard) {
        statusCard.className = 'card-value ' + (data.status === 'ok' ? 'ok' : 'error');
      }
    } catch (e) {
      mnemo.setText('health-label', 'offline');
      const dot = document.getElementById('health-dot');
      if (dot) dot.className = 'health-dot error';
    }
  }, 5000);

  // Ops summary polling (every 10s)
  mnemo.poll(async () => {
    try {
      const data = await mnemo.api('GET', '/api/v1/ops/summary');
      mnemo.setText('card-http-requests', String(data.http_requests_total || 0));
      mnemo.setText('card-webhooks', String(data.webhook_deliveries_total || 0));
      const dl = data.webhook_dead_letters_total || 0;
      const dlEl = document.getElementById('card-dead-letter');
      if (dlEl) {
        dlEl.textContent = String(dl);
        dlEl.className = 'card-value' + (dl > 0 ? ' warn' : '');
      }
      const viol = data.policy_violations_total || 0;
      const violEl = document.getElementById('card-violations');
      if (violEl) {
        violEl.textContent = String(viol);
        violEl.className = 'card-value' + (viol > 0 ? ' error' : '');
      }
    } catch (e) {
      // ops summary not critical
    }
  }, 10000);

  // Webhook list polling (every 15s)
  mnemo.poll(async () => {
    try {
      const data = await mnemo.api('GET', '/api/v1/memory/webhooks');
      const hooks = data.data || [];
      if (hooks.length === 0) {
        mnemo.setHtml('webhook-status-panel', '<p class="muted">No webhooks registered.</p>');
        return;
      }
      let rows = hooks.map(h => {
        const status = h.enabled ? '<span class="status-ok">enabled</span>' : '<span class="status-warn">disabled</span>';
        const events = escapeHtml((h.events || []).join(', '));
        return `<tr>
          <td><code>${escapeHtml(h.id.substring(0, 8))}</code></td>
          <td>${escapeHtml(h.target_url)}</td>
          <td>${status}</td>
          <td>${escapeHtml(h.user_identifier)}</td>
          <td style="font-size:11px;color:var(--text-muted)">${events}</td>
        </tr>`;
      }).join('');
      mnemo.setHtml('webhook-status-panel', `<table>
        <thead><tr><th>ID</th><th>Target</th><th>Status</th><th>User</th><th>Events</th></tr></thead>
        <tbody>${rows}</tbody>
      </table>`);
    } catch (e) {
      mnemo.setHtml('webhook-status-panel', '<p class="muted">Could not load webhooks.</p>');
    }
  }, 15000);
}

// ─── Webhooks page ─────────────────────────────────────────────────
// (Phase B — will be expanded)

// ─── RCA page ──────────────────────────────────────────────────────
// (Phase B — will be expanded)

// ─── Governance page ───────────────────────────────────────────────
// (Phase B — will be expanded)

// ─── Traces page ───────────────────────────────────────────────────
// (Phase B — will be expanded)

// ─── Explorer page ─────────────────────────────────────────────────
// (Phase B — will be expanded)

// ─── Utilities ─────────────────────────────────────────────────────
function escapeHtml(str) {
  const div = document.createElement('div');
  div.textContent = str || '';
  return div.innerHTML;
}

// ─── Boot ──────────────────────────────────────────────────────────
document.addEventListener('DOMContentLoaded', () => {
  initNav();
  initHome();
});
