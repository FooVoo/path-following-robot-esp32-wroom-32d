//! UI handlers — serve embedded HTML pages.

use axum::{extract::Path, response::Html};

const FLEET_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Robot Fleet</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:system-ui,-apple-system,sans-serif;background:#0d1117;color:#c9d1d9;padding:1.5rem 2rem;min-height:100vh}
h1{color:#58a6ff;margin-bottom:.4rem}
.meta{color:#8b949e;font-size:.85rem;margin-bottom:1.25rem;display:flex;gap:1rem;align-items:center}
.badge{padding:2px 8px;border-radius:4px;font-size:.72rem;font-weight:700;letter-spacing:.4px}
.s-IDLE{background:#21262d;color:#8b949e}.s-RECORD{background:#3d2b00;color:#e3b341}
.s-READY{background:#0c2d6b;color:#58a6ff}.s-PLAY{background:#0f2d1a;color:#3fb950}
.s-AVOIDING{background:#3d0c0c;color:#f85149}.s-HALT{background:#500c0c;color:#ffa198}
table{width:100%;border-collapse:collapse;background:#161b22;border-radius:6px;overflow:hidden}
th{padding:9px 14px;text-align:left;font-size:.75rem;color:#8b949e;text-transform:uppercase;
   letter-spacing:.4px;background:#1c2128;border-bottom:1px solid #30363d}
td{padding:9px 14px;border-bottom:1px solid #21262d;font-size:.875rem}
tr:hover td{background:#1c2128}tr:last-child td{border-bottom:none}
a{color:#58a6ff;text-decoration:none}a:hover{text-decoration:underline}
.dot{width:8px;height:8px;border-radius:50%;display:inline-block;margin-right:6px;vertical-align:middle}
.online{background:#3fb950;box-shadow:0 0 6px #3fb95088}.offline{background:#484f58}
.dim{color:#8b949e}
.conn{font-size:.8rem;padding:3px 10px;border-radius:4px}
.live{background:#0f2d1a;color:#3fb950}.dead{background:#3d0c0c;color:#f85149}
#empty{text-align:center;padding:3rem;color:#484f58;display:none;font-size:.95rem}
</style>
</head>
<body>
<h1>🤖 Robot Fleet</h1>
<div class="meta">
  <span id="conn" class="conn dead">● Connecting…</span>
  <span id="meta-count" class="dim"></span>
</div>
<table>
<thead>
<tr><th>Robot</th><th>State</th><th>LIDAR L</th><th>LIDAR R</th>
    <th>Throttle L/R</th><th>Uptime</th><th>Last Seen</th></tr>
</thead>
<tbody id="tbody"></tbody>
</table>
<p id="empty">No robots seen yet — waiting for telemetry…</p>
<script>
const robots={},tbody=document.getElementById('tbody'),empty=document.getElementById('empty'),
      connEl=document.getElementById('conn'),metaEl=document.getElementById('meta-count');
function fmtU(ms){const s=Math.floor(ms/1e3),m=Math.floor(s/60),h=Math.floor(m/60);
  return h?`${h}h ${m%60}m`:m?`${m}m ${s%60}s`:`${s}s`}
function fmtA(iso){const d=Math.floor((Date.now()-new Date(iso))/1e3);
  return d<5?'just now':d<60?`${d}s ago`:d<3600?`${Math.floor(d/60)}m ago`:`${Math.floor(d/3600)}h ago`}
function isOnline(iso){return Date.now()-new Date(iso)<10000}
function row(ip,{frame:f,ts}){
  const ll=f.lidar_left_cm??-1,lr=f.lidar_right_cm??-1;
  return `<td><span class="dot ${isOnline(ts)?'online':'offline'}"></span>
    <a href="/robots/${ip}">${ip}</a></td>
    <td><span class="badge s-${f.state}">${f.state}</span></td>
    <td>${ll<0?'<span class="dim">—</span>':ll+' cm'}</td>
    <td>${lr<0?'<span class="dim">—</span>':lr+' cm'}</td>
    <td>${f.throttle_left} / ${f.throttle_right}</td><td>${fmtU(f.uptime_ms)}</td>
    <td class="dim">${fmtA(ts)}</td>`}
function upsert(ip,data){
  robots[ip]=data;
  const id='r-'+ip.replace(/\./g,'-');
  let tr=document.getElementById(id);
  if(!tr){tr=document.createElement('tr');tr.id=id;tbody.appendChild(tr);empty.style.display='none'}
  tr.innerHTML=row(ip,data);
  metaEl.textContent=`${Object.keys(robots).length} robot(s)`}
const es=new EventSource('/events');
es.addEventListener('telemetry',e=>{const d=JSON.parse(e.data);upsert(d.robot_id,{frame:d.frame,ts:d.timestamp})});
es.onopen=()=>{connEl.className='conn live';connEl.textContent='● Live'};
es.onerror=()=>{connEl.className='conn dead';connEl.textContent='● Reconnecting…'};
setInterval(()=>Object.keys(robots).forEach(ip=>{
  const tr=document.getElementById('r-'+ip.replace(/\./g,'-'));
  if(tr)tr.innerHTML=row(ip,robots[ip])}),5000);
</script>
</body>
</html>"#;

const ROBOT_HTML_TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Robot {ROBOT_ID}</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:system-ui,-apple-system,sans-serif;background:#0d1117;color:#c9d1d9;padding:1.5rem 2rem}
a{color:#58a6ff;text-decoration:none}a:hover{text-decoration:underline}
h1{color:#58a6ff;margin:.6rem 0 .25rem;font-size:1.4rem}
h2{color:#c9d1d9;font-size:.9rem;font-weight:600;text-transform:uppercase;
   letter-spacing:.5px;color:#8b949e;margin:1.25rem 0 .5rem}
.badge{padding:3px 10px;border-radius:4px;font-size:.78rem;font-weight:700}
.s-IDLE{background:#21262d;color:#8b949e}.s-RECORD{background:#3d2b00;color:#e3b341}
.s-READY{background:#0c2d6b;color:#58a6ff}.s-PLAY{background:#0f2d1a;color:#3fb950}
.s-AVOIDING{background:#3d0c0c;color:#f85149}.s-HALT{background:#500c0c;color:#ffa198}
.panel{background:#161b22;border:1px solid #30363d;border-radius:6px;
       padding:1rem 1.25rem;display:flex;gap:2rem;flex-wrap:wrap;margin:.75rem 0}
.stat{display:flex;flex-direction:column;gap:3px}
.slabel{font-size:.68rem;text-transform:uppercase;color:#8b949e;letter-spacing:.4px}
.sval{font-size:1rem;font-weight:500}
table{width:100%;border-collapse:collapse;background:#161b22;border-radius:6px;overflow:hidden}
th{padding:8px 12px;text-align:left;font-size:.72rem;color:#8b949e;text-transform:uppercase;
   letter-spacing:.4px;background:#1c2128;border-bottom:1px solid #30363d}
td{padding:7px 12px;border-bottom:1px solid #21262d;font-size:.8rem;font-family:ui-monospace,monospace}
tr:hover td{background:#1c2128}tr:last-child td{border-bottom:none}
.dim{color:#8b949e}
.controls{display:flex;gap:.6rem;align-items:center;margin-bottom:.5rem}
button{background:#21262d;border:1px solid #30363d;color:#c9d1d9;padding:5px 12px;
       border-radius:4px;cursor:pointer;font-size:.82rem}
button:hover{background:#30363d}button:disabled{opacity:.4;cursor:default}
#loginfo{color:#8b949e;font-size:.82rem}
</style>
</head>
<body>
<a href="/">← Fleet</a>
<h1>🤖 {ROBOT_ID}</h1>

<div class="panel">
  <div class="stat"><span class="slabel">State</span>
    <span id="ls" class="sval dim">—</span></div>
  <div class="stat"><span class="slabel">LIDAR Left</span>
    <span id="lll" class="sval dim">—</span></div>
  <div class="stat"><span class="slabel">LIDAR Right</span>
    <span id="llr" class="sval dim">—</span></div>
  <div class="stat"><span class="slabel">Throttle L/R</span>
    <span id="lth" class="sval dim">—</span></div>
  <div class="stat"><span class="slabel">Uptime</span>
    <span id="lup" class="sval dim">—</span></div>
  <div class="stat"><span class="slabel">Last Seen</span>
    <span id="lsn" class="sval dim">—</span></div>
</div>

<h2>Telemetry Log</h2>
<div class="controls">
  <button id="btn-more" onclick="loadMore()">Load older</button>
  <span id="loginfo"></span>
</div>
<table>
<thead>
<tr><th>Time</th><th>State</th><th>LIDAR L</th><th>LIDAR R</th>
    <th>Thr L/R</th><th>Uptime ms</th></tr>
</thead>
<tbody id="ltbody"></tbody>
</table>

<script>
const IP='{ROBOT_ID}';
const tbody=document.getElementById('ltbody'),
      info=document.getElementById('loginfo'),
      btnMore=document.getElementById('btn-more');
let offset=0,total=0;
const LIMIT=100;

function fmtU(ms){const s=Math.floor(ms/1e3),m=Math.floor(s/60),h=Math.floor(m/60);
  return h?`${h}h ${m%60}m`:m?`${m}m ${s%60}s`:`${s}s`}
function fmtA(iso){const d=Math.floor((Date.now()-new Date(iso))/1e3);
  return d<5?'just now':d<60?`${d}s ago`:d<3600?`${Math.floor(d/60)}m ago`:`${Math.floor(d/3600)}h ago`}
function fmtT(iso){const d=new Date(iso);
  return d.toLocaleTimeString()+'.'+String(d.getMilliseconds()).padStart(3,'0')}
function logRow({received_at,frame:f}){
  const ll=f.lidar_left_cm??-1,lr=f.lidar_right_cm??-1;
  return `<tr>
    <td>${fmtT(received_at)}</td>
    <td><span class="badge s-${f.state}">${f.state}</span></td>
    <td>${ll<0?'<span class="dim">—</span>':ll+' cm'}</td>
    <td>${lr<0?'<span class="dim">—</span>':lr+' cm'}</td>
    <td>${f.throttle_left} / ${f.throttle_right}</td>
    <td class="dim">${f.uptime_ms}</td></tr>`}

async function loadMore(){
  btnMore.disabled=true;
  const r=await fetch(`/robots/${encodeURIComponent(IP)}/logs?limit=${LIMIT}&offset=${offset}`);
  const d=await r.json();
  if(d.error){info.textContent='DB unavailable — no stored history';btnMore.style.display='none';return}
  total=d.total;offset+=d.logs.length;
  tbody.insertAdjacentHTML('beforeend',d.logs.map(logRow).join(''));
  info.textContent=`${offset} of ${total} entries`;
  btnMore.disabled=offset>=total}

const es=new EventSource('/events');
es.addEventListener('telemetry',e=>{
  const d=JSON.parse(e.data);
  if(d.robot_id!==IP)return;
  const f=d.frame;
  document.getElementById('ls').innerHTML=`<span class="badge s-${f.state}">${f.state}</span>`;
  document.getElementById('lll').textContent=(f.lidar_left_cm??-1)<0?'—':(f.lidar_left_cm)+' cm';
  document.getElementById('llr').textContent=(f.lidar_right_cm??-1)<0?'—':(f.lidar_right_cm)+' cm';
  document.getElementById('lth').textContent=`${f.throttle_left} / ${f.throttle_right}`;
  document.getElementById('lup').textContent=fmtU(f.uptime_ms);
  document.getElementById('lsn').textContent=fmtA(d.timestamp);
  tbody.insertAdjacentHTML('afterbegin',logRow({received_at:d.timestamp,frame:f}));
  if(offset>0){offset++;info.textContent=`${offset} of ${total} entries`}});

loadMore();
</script>
</body>
</html>"#;

pub async fn fleet_ui() -> Html<&'static str> {
    Html(FLEET_HTML)
}

pub async fn robot_ui(Path(id): Path<String>) -> Html<String> {
    Html(ROBOT_HTML_TEMPLATE.replace("{ROBOT_ID}", &id))
}
