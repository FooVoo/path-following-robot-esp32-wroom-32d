/** Format milliseconds as "Xh Ym", "Xm Ys", or "Xs". */
function fmtU(ms) {
  const s = Math.floor(ms / 1e3);
  const m = Math.floor(s / 60);
  const h = Math.floor(m / 60);
  return h ? `${h}h ${m % 60}m`
           : m ? `${m}m ${s % 60}s`
               : `${s}s`;
}

/** Format an ISO timestamp as a human-readable "time ago" string. */
function fmtA(iso) {
  const d = Math.floor((Date.now() - new Date(iso)) / 1e3);
  return d < 5    ? 'just now'
       : d < 60   ? `${d}s ago`
       : d < 3600 ? `${Math.floor(d / 60)}m ago`
                  : `${Math.floor(d / 3600)}h ago`;
}
