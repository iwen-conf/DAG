/* sunmao Web UI — human console; no LLM. Same-origin /v1 API. */
(() => {
  const $ = (sel) => document.querySelector(sel);
  const $$ = (sel) => [...document.querySelectorAll(sel)];

  const state = {
    projects: [],
    projectId: null,
    graph: null,
    selectedNodeId: null,
    sse: null,
  };

  function actor() {
    return ($("#actor").value || "human").trim() || "human";
  }

  function toast(msg, isError = false) {
    const el = $("#toast");
    el.textContent = msg;
    el.classList.toggle("error", isError);
    el.classList.remove("hidden");
    clearTimeout(toast._t);
    toast._t = setTimeout(() => el.classList.add("hidden"), 3200);
  }

  async function api(method, path, body) {
    const opts = {
      method,
      headers: {
        "X-Sunmao-Actor": actor(),
        Accept: "application/json",
      },
    };
    if (body !== undefined) {
      opts.headers["Content-Type"] = "application/json";
      opts.body = JSON.stringify(body);
    }
    const res = await fetch(path, opts);
    const text = await res.text();
    let data = null;
    if (text) {
      try {
        data = JSON.parse(text);
      } catch {
        data = { raw: text };
      }
    }
    if (!res.ok) {
      const msg = data?.message || data?.code || res.statusText;
      const err = new Error(`${method} ${path} → ${res.status}: ${msg}`);
      err.status = res.status;
      err.data = data;
      throw err;
    }
    return { status: res.status, data };
  }

  async function loadProjects() {
    const { data } = await api("GET", "/v1/projects");
    state.projects = data.projects || [];
    renderProjects();
  }

  function renderProjects() {
    const ul = $("#project-list");
    const empty = $("#projects-empty");
    ul.innerHTML = "";
    empty.classList.toggle("hidden", state.projects.length > 0);
    for (const p of state.projects) {
      const li = document.createElement("li");
      li.dataset.id = p.id;
      if (p.id === state.projectId) li.classList.add("active");
      li.innerHTML = `<span class="name"></span><span class="id mono"></span>`;
      li.querySelector(".name").textContent = p.name;
      li.querySelector(".id").textContent = p.id;
      li.addEventListener("click", () => selectProject(p.id));
      ul.appendChild(li);
    }
  }

  async function selectProject(pid) {
    state.projectId = pid;
    state.selectedNodeId = null;
    renderProjects();
    $("#welcome").classList.add("hidden");
    $("#project-view").classList.remove("hidden");
    const p = state.projects.find((x) => x.id === pid);
    if (p) {
      $("#pv-name").textContent = p.name;
      $("#pv-id").textContent = p.id;
      $("#pv-repo").textContent = p.repo_path || "";
    } else {
      const { data } = await api("GET", `/v1/projects/${pid}`);
      $("#pv-name").textContent = data.name;
      $("#pv-id").textContent = data.id;
      $("#pv-repo").textContent = data.repo_path || "";
    }
    await loadGraph();
    if ($("#tab-events").classList.contains("hidden") === false) {
      await loadEvents();
    }
  }

  async function loadGraph() {
    if (!state.projectId) return;
    const { data } = await api("GET", `/v1/projects/${state.projectId}/graph`);
    state.graph = data;
    $("#graph-version").textContent = `version ${data.version ?? "—"}`;
    renderTree();
    if (state.selectedNodeId) {
      renderNodeDetail(state.selectedNodeId);
    } else {
      $("#node-detail").innerHTML = `<p class="muted">选择节点查看详情与操作</p>`;
    }
  }

  function pathDepth(path) {
    if (!path) return 0;
    return path.split(".").length - 1;
  }

  function renderTree() {
    const root = $("#node-tree");
    root.innerHTML = "";
    const nodes = [...(state.graph?.nodes || [])].sort((a, b) =>
      (a.path || a.id).localeCompare(b.path || b.id)
    );
    if (!nodes.length) {
      root.innerHTML = `<p class="muted" style="padding:0.75rem">图为空。由外部 Planner 调用 publish 填入。</p>`;
      return;
    }
    for (const n of nodes) {
      const depth = pathDepth(n.path);
      const el = document.createElement("div");
      el.className = "tree-item" + (n.id === state.selectedNodeId ? " active" : "");
      el.dataset.id = n.id;
      const st = n.task_state || n.scope_state || "-";
      el.innerHTML = `
        <span class="indent" style="width:${depth * 14}px"></span>
        <span class="badge ${n.kind}"></span>
        <span class="title"></span>
        <span class="badge ${st}"></span>
      `;
      el.querySelector(".badge." + n.kind).textContent = n.kind;
      el.querySelector(".title").textContent = n.title || n.id;
      el.querySelectorAll(".badge")[1].textContent = st;
      el.addEventListener("click", () => {
        state.selectedNodeId = n.id;
        renderTree();
        renderNodeDetail(n.id);
      });
      root.appendChild(el);
    }
  }

  function findNode(id) {
    return (state.graph?.nodes || []).find((n) => n.id === id);
  }

  function renderNodeDetail(id) {
    const n = findNode(id);
    const box = $("#node-detail");
    if (!n) {
      box.innerHTML = `<p class="muted">节点不存在</p>`;
      return;
    }
    const edges = (state.graph?.edges || []).filter(
      (e) => e.from === id || e.to === id
    );
    box.innerHTML = `
      <h3></h3>
      <div class="kv mono">
        <div><span class="muted">id</span> <span data-k="id"></span></div>
        <div><span class="muted">kind</span> <span data-k="kind"></span></div>
        <div><span class="muted">path</span> <span data-k="path"></span></div>
        <div><span class="muted">state</span> <span data-k="state"></span></div>
        <div><span class="muted">ready</span> <span data-k="ready"></span></div>
        <div><span class="muted">needs_replan</span> <span data-k="nr"></span></div>
        <div><span class="muted">write_scope</span> <span data-k="ws"></span></div>
      </div>
      <div class="actions" id="node-actions"></div>
      <h4 class="muted" style="margin:1rem 0 0.35rem;font-weight:500">关联边</h4>
      <pre id="node-edges"></pre>
    `;
    box.querySelector("h3").textContent = n.title || n.id;
    box.querySelector('[data-k="id"]').textContent = n.id;
    box.querySelector('[data-k="kind"]').textContent = n.kind;
    box.querySelector('[data-k="path"]').textContent = n.path || "—";
    box.querySelector('[data-k="state"]').textContent =
      n.task_state || n.scope_state || "—";
    box.querySelector('[data-k="ready"]').textContent = String(!!n.ready);
    box.querySelector('[data-k="nr"]').textContent = String(!!n.needs_replan);
    box.querySelector('[data-k="ws"]').textContent = (n.write_scope || []).join(", ") || "—";
    box.querySelector("#node-edges").textContent =
      edges.length ? JSON.stringify(edges, null, 2) : "(none)";

    const actions = box.querySelector("#node-actions");
    if (n.kind === "package") {
      for (const action of ["pause", "close", "reopen", "archive"]) {
        const b = document.createElement("button");
        b.type = "button";
        b.className = "btn small" + (action === "close" ? " danger" : "");
        b.textContent = `scope ${action}`;
        b.addEventListener("click", () => scopeAction(n.id, action));
        actions.appendChild(b);
      }
      const force = document.createElement("button");
      force.type = "button";
      force.className = "btn small danger";
      force.textContent = "close --force";
      force.addEventListener("click", () => scopeAction(n.id, "close", true));
      actions.appendChild(force);
    }
    if (n.kind === "task") {
      const cancel = document.createElement("button");
      cancel.type = "button";
      cancel.className = "btn small danger";
      cancel.textContent = "cancel";
      cancel.addEventListener("click", async () => {
        try {
          await api("POST", `/v1/projects/${state.projectId}/tasks/${n.id}/cancel`, {
            force: false,
          });
          toast("task cancelled");
          await loadGraph();
        } catch (e) {
          toast(e.message, true);
        }
      });
      actions.appendChild(cancel);
      const detail = document.createElement("button");
      detail.type = "button";
      detail.className = "btn small";
      detail.textContent = "GET task";
      detail.addEventListener("click", async () => {
        try {
          const { data } = await api(
            "GET",
            `/v1/projects/${state.projectId}/tasks/${n.id}`
          );
          box.querySelector("#node-edges").textContent = JSON.stringify(data, null, 2);
        } catch (e) {
          toast(e.message, true);
        }
      });
      actions.appendChild(detail);
    }
  }

  async function scopeAction(packageId, action, force = false) {
    const reason = prompt(`scope ${action} 原因`, "web-ui") || "web-ui";
    try {
      await api("POST", `/v1/projects/${state.projectId}/packages/${packageId}/scope`, {
        action,
        reason,
        force,
        permanent: false,
      });
      toast(`scope → ${action}`);
      await loadGraph();
    } catch (e) {
      toast(e.message, true);
    }
  }

  async function loadEvents() {
    if (!state.projectId) return;
    const after = Number($("#after-seq").value || 0);
    const { data } = await api(
      "GET",
      `/v1/projects/${state.projectId}/events?after_seq=${after}&limit=100`
    );
    const list = $("#event-list");
    list.innerHTML = "";
    const events = data.events || [];
    if (!events.length) {
      list.innerHTML = `<p class="muted" style="padding:0.75rem">无事件（after_seq=${after}）</p>`;
      return;
    }
    for (const ev of events) {
      appendEvent(ev);
    }
  }

  function appendEvent(ev) {
    const list = $("#event-list");
    const empty = list.querySelector(".muted");
    if (empty) list.innerHTML = "";
    const div = document.createElement("div");
    div.className = "event";
    div.innerHTML = `<div><span class="kind"></span> <span class="muted mono seq"></span></div>
      <div class="muted mono node"></div>
      <pre class="payload"></pre>`;
    div.querySelector(".kind").textContent = ev.kind || "event";
    div.querySelector(".seq").textContent = `seq=${ev.seq ?? "?"}`;
    div.querySelector(".node").textContent = ev.node_id || "(graph)";
    div.querySelector(".payload").textContent = JSON.stringify(
      ev.payload ?? ev,
      null,
      2
    );
    list.prepend(div);
  }

  function stopSse() {
    if (state.sse) {
      state.sse.close();
      state.sse = null;
    }
  }

  function startSse() {
    stopSse();
    if (!state.projectId) return;
    const after = Number($("#after-seq").value || 0);
    // EventSource cannot set custom headers; use fetch stream fallback via polyfill pattern:
    // Browser EventSource won't send X-Sunmao-Actor → 401.
    // Use fetch + ReadableStream for SSE with headers.
    const ctrl = new AbortController();
    state.sse = { close: () => ctrl.abort() };
    (async () => {
      try {
        const res = await fetch(
          `/v1/projects/${state.projectId}/events/stream?after_seq=${after}`,
          {
            headers: { "X-Sunmao-Actor": actor(), Accept: "text/event-stream" },
            signal: ctrl.signal,
          }
        );
        if (!res.ok) throw new Error(`SSE ${res.status}`);
        const reader = res.body.getReader();
        const dec = new TextDecoder();
        let buf = "";
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          buf += dec.decode(value, { stream: true });
          const parts = buf.split("\n\n");
          buf = parts.pop() || "";
          for (const block of parts) {
            const dataLine = block
              .split("\n")
              .find((l) => l.startsWith("data:"));
            if (!dataLine) continue;
            try {
              const payload = JSON.parse(dataLine.slice(5).trim());
              appendEvent(payload);
              if (payload.seq != null) {
                $("#after-seq").value = String(payload.seq);
              }
            } catch {
              /* ignore parse */
            }
          }
        }
      } catch (e) {
        if (e.name !== "AbortError") toast(`SSE: ${e.message}`, true);
      }
    })();
  }

  function switchTab(name) {
    $$(".tab").forEach((t) =>
      t.classList.toggle("active", t.dataset.tab === name)
    );
    $("#tab-tree").classList.toggle("hidden", name !== "tree");
    $("#tab-events").classList.toggle("hidden", name !== "events");
    $("#tab-admin").classList.toggle("hidden", name !== "admin");
    if (name === "events") loadEvents().catch((e) => toast(e.message, true));
    if (name !== "events") stopSse();
  }

  // bindings
  $("#btn-refresh").addEventListener("click", () => {
    loadProjects()
      .then(() => (state.projectId ? selectProject(state.projectId) : null))
      .catch((e) => toast(e.message, true));
  });
  $("#btn-new-project").addEventListener("click", () => {
    $("#project-create").classList.toggle("hidden");
  });
  $("#btn-create-project").addEventListener("click", async () => {
    const name = $("#new-name").value.trim();
    const repo_path = $("#new-repo").value.trim();
    if (!name || !repo_path) {
      toast("需要 name 与 repo_path", true);
      return;
    }
    try {
      const { data } = await api("POST", "/v1/projects", { name, repo_path });
      toast(`项目 ${data.name}`);
      await loadProjects();
      await selectProject(data.id);
      $("#project-create").classList.add("hidden");
    } catch (e) {
      toast(e.message, true);
    }
  });
  $("#btn-reload-graph").addEventListener("click", () =>
    loadGraph().catch((e) => toast(e.message, true))
  );
  $("#btn-load-events").addEventListener("click", () =>
    loadEvents().catch((e) => toast(e.message, true))
  );
  $("#sse-toggle").addEventListener("change", (e) => {
    if (e.target.checked) startSse();
    else stopSse();
  });
  $$(".tab").forEach((t) =>
    t.addEventListener("click", () => switchTab(t.dataset.tab))
  );
  $("#btn-rebuild").addEventListener("click", async () => {
    try {
      const { data } = await api(
        "POST",
        `/v1/projects/${state.projectId}/admin/rebuild-projection`,
        {}
      );
      $("#admin-out").textContent = JSON.stringify(data, null, 2);
      toast("rebuild ok");
      await loadGraph();
    } catch (e) {
      toast(e.message, true);
    }
  });
  $("#btn-verify").addEventListener("click", async () => {
    try {
      const { data } = await api(
        "POST",
        `/v1/projects/${state.projectId}/admin/verify`,
        {}
      );
      $("#admin-out").textContent = JSON.stringify(data, null, 2);
    } catch (e) {
      toast(e.message, true);
    }
  });
  $("#btn-approve-major").addEventListener("click", async () => {
    const id = $("#artifact-id").value.trim();
    if (!id) {
      toast("需要 artifact id", true);
      return;
    }
    try {
      const { data } = await api(
        "POST",
        `/v1/projects/${state.projectId}/contracts/${id}/approve-major`,
        {}
      );
      $("#admin-out").textContent = JSON.stringify(data, null, 2);
      toast("approved");
      await loadGraph();
    } catch (e) {
      toast(e.message, true);
    }
  });

  loadProjects().catch((e) => toast(e.message, true));
})();
