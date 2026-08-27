#!/usr/bin/env python3
"""Strict per-widget visual verification of the Vox Trader design reference.

Renders frontend/design-system/reference/index.html at 1280/1440/1920 in Compact,
Standard and Comfortable and checks every widget for nine classes of defect. It measures
*painted* geometry — a layout rectangle intersected with every clipping ancestor, and
per-line boxes for inline elements — because a layout rect that sits outside a scroller is
not painted there, and an inline element wrapped over two lines reports a rectangle that
covers both.

Four known defects are then injected on purpose. If any of them fails to be detected the
run fails: a zero that cannot fail is not evidence.

Exit code 0 only when the clean matrix is empty and every negative control fires.
"""
import argparse
import json
import os
import pathlib
import sys

from playwright.sync_api import sync_playwright

WIDTHS = (1280, 1440, 1920)
DENSITIES = ("compact", "standard", "comfortable")

# Expected control/row/widget-header height per density, from the density table.
GEOMETRY = {"compact": (28, 26, 32), "standard": (32, 30, 36), "comfortable": (36, 36, 36)}

NEGATIVE_CONTROLS = (
    ("column gutter removed", ".vox-table__header,.vox-table__row{column-gap:0 !important}", "columns-touching"),
    ("execution target row pinned to one line",
     ".vox-ticket__target{height:26px !important;min-height:0 !important;overflow:hidden}", "clip-y"),
    ("table header without its leading border", ".vox-table__header{border-left-width:0 !important}", "column-misalign"),
    ("numeric control squeezed", ".vox-numeric{width:60px !important}", "control-too-narrow"),
)

PROBE = r"""
() => {
  const sec = el => { const s = el.closest('section.ref__sec'); return s ? s.id : '(none)'; };
  const cls = el => (el.className && el.className.toString ? el.className.toString() : '').slice(0, 60);
  const txt = el => (el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 44);
  const isScroller = el => { const cs = getComputedStyle(el); return /auto|scroll/.test(cs.overflowX) || /auto|scroll/.test(cs.overflowY); };
  const visible = el => { const r = el.getBoundingClientRect(); const cs = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && cs.visibility !== 'hidden' && cs.display !== 'none' && cs.opacity !== '0'; };
  const inRef = el => el.closest('.ref');
  const findings = [];
  const add = (kind, el, detail) => findings.push({ kind, sec: sec(el), cls: cls(el), text: txt(el), detail });

  // Painted geometry: the layout rect clipped by every ancestor that clips.
  const clipsContent = cs => /hidden|auto|scroll|clip/.test(cs.overflowX) || /hidden|auto|scroll|clip/.test(cs.overflowY);
  const vrect = el => {
    let r = el.getBoundingClientRect();
    let box = { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
    for (let a = el.parentElement; a && a !== document.body; a = a.parentElement) {
      const cs = getComputedStyle(a);
      if (!clipsContent(cs)) continue;
      const ra = a.getBoundingClientRect();
      box.left = Math.max(box.left, ra.left); box.top = Math.max(box.top, ra.top);
      box.right = Math.min(box.right, ra.right); box.bottom = Math.min(box.bottom, ra.bottom);
    }
    box.width = box.right - box.left; box.height = box.bottom - box.top;
    return box;
  };

  const all = [...document.querySelectorAll('.ref *')].filter(visible);

  // 1. horizontal clipping outside a scroll container
  for (const el of all) {
    if (el.scrollWidth > el.clientWidth + 1 && !isScroller(el)) add('clip-x', el, el.scrollWidth + '>' + el.clientWidth);
  }

  // 2. vertical clipping: a child's bottom edge outside the padding box.
  //    scrollHeight counts the container's own borders, so measure geometry instead.
  for (const el of all) {
    const cs = getComputedStyle(el);
    if (isScroller(el) || cs.overflowY !== 'hidden') continue;
    const r = el.getBoundingClientRect();
    const bb = parseFloat(cs.borderBottomWidth) || 0;
    const innerBottom = r.bottom - bb;
    for (const c of el.children) {
      const rc = c.getBoundingClientRect();
      if (rc.height > 0 && rc.bottom > innerBottom + 1.5) {
        add('clip-y', el, cls(c) + ' overflows by ' + Math.round(rc.bottom - innerBottom) + 'px');
        break;
      }
    }
  }

  // 3. truncation of a capital-affecting value
  const capital = '.vox-num, .vox-ticket__target-name, .vox-account__label, .vox-account-row__name,' +
    '.vox-protect__result-value, .vox-trailing__state-value, .vox-migrate__policy-value,' +
    '.vox-recon__fact-value, .vox-metrics__value, .vox-exec-fact__state';
  for (const el of document.querySelectorAll(capital)) {
    if (!visible(el) || !inRef(el)) continue;
    if (el.scrollWidth > el.clientWidth + 1) add('truncated-capital', el, el.scrollWidth + '>' + el.clientWidth);
  }

  // 4. text overlap, compared per painted line box
  const leaves = all.filter(el => el.children.length === 0 && (el.textContent || '').trim().length > 1);
  const clipTo = (r, el) => {
    const v = vrect(el);
    const b = { left: Math.max(r.left, v.left), top: Math.max(r.top, v.top),
                right: Math.min(r.right, v.right), bottom: Math.min(r.bottom, v.bottom) };
    b.width = b.right - b.left; b.height = b.bottom - b.top; return b;
  };
  const cache = new Map();
  const boxesOf = el => {
    if (!cache.has(el)) cache.set(el, [...el.getClientRects()].map(r => clipTo(r, el)).filter(r => r.width > 1 && r.height > 1));
    return cache.get(el);
  };
  for (let i = 0; i < leaves.length; i++) {
    const a = leaves[i], ra = vrect(a);
    if (ra.width <= 1 || ra.height <= 1) continue;
    for (let j = i + 1; j < leaves.length; j++) {
      const b = leaves[j], rb = vrect(b);
      if (rb.width <= 1 || rb.height <= 1) continue;
      if (rb.top > ra.bottom) break;
      if (a.contains(b) || b.contains(a)) continue;
      const pa = getComputedStyle(a).position, pb = getComputedStyle(b).position;
      if (pa === 'absolute' || pb === 'absolute') continue;
      let ox = 0, oy = 0;
      for (const la of boxesOf(a)) for (const lb of boxesOf(b)) {
        const x = Math.min(la.right, lb.right) - Math.max(la.left, lb.left);
        const y = Math.min(la.bottom, lb.bottom) - Math.max(la.top, lb.top);
        if (x > 2 && y > 2 && x * y > ox * oy) { ox = x; oy = y; }
      }
      if (ox > 2 && oy > 2) {
        findings.push({ kind: 'text-overlap', sec: sec(a), cls: cls(a) + ' x ' + cls(b),
                        text: txt(a) + ' x ' + txt(b), detail: Math.round(ox) + 'x' + Math.round(oy) + 'px' });
      }
    }
  }

  // 5. adjacent cells touching with no gutter, which reads as one run-together word
  for (const row of document.querySelectorAll('.ref .vox-table__header, .ref .vox-table__row')) {
    const cells = [...row.children].filter(visible);
    for (let i = 0; i + 1 < cells.length; i++) {
      const ta = (cells[i].textContent || '').trim(), tb = (cells[i + 1].textContent || '').trim();
      if (!ta || !tb) continue;
      const range = document.createRange();
      range.selectNodeContents(cells[i]);
      const inkRight = range.getBoundingClientRect().right;
      range.selectNodeContents(cells[i + 1]);
      const inkLeft = range.getBoundingClientRect().left;
      if (inkLeft - inkRight < 3) add('columns-touching', row, '"' + ta.slice(-12) + '" | "' + tb.slice(0, 12) + '"');
    }
  }

  // 6. content reaching into a scrollbar gutter
  for (const el of all) {
    if (!isScroller(el)) continue;
    const gutterY = el.offsetWidth - el.clientWidth;
    if (gutterY < 2) continue;
    const r = el.getBoundingClientRect();
    for (const c of el.children) {
      const rc = c.getBoundingClientRect();
      if (rc.right > r.right - gutterY + 1 && (c.textContent || '').trim())
        add('scrollbar-collision', c, 'reaches the ' + Math.round(gutterY) + 'px gutter');
    }
  }

  // 7. table column alignment
  for (const table of document.querySelectorAll('.ref .vox-table')) {
    const head = table.querySelector('.vox-table__header');
    const row = table.querySelector('.vox-table__row');
    if (!head || !row || head.children.length !== row.children.length) continue;
    for (let i = 0; i < head.children.length; i++) {
      const dh = head.children[i].getBoundingClientRect();
      const dr = row.children[i].getBoundingClientRect();
      if (Math.abs(dh.left - dr.left) > 1.5)
        add('column-misalign', table, 'col ' + i + ' header@' + Math.round(dh.left) + ' row@' + Math.round(dr.left));
    }
  }

  // 8. controls too small to hold their value
  for (const el of document.querySelectorAll('.ref .vox-numeric, .ref .vox-input, .ref .vox-select, .ref .vox-btn, .ref .vox-segmented')) {
    if (!visible(el)) continue;
    const w = el.getBoundingClientRect().width;
    const min = el.classList.contains('vox-numeric') ? 92 : 56;
    if (w < min) add('control-too-narrow', el, Math.round(w) + 'px < ' + min);
  }

  // 9. explanatory copy landing on tabular values
  for (const note of document.querySelectorAll('.ref .ref__note, .ref .vox-protect__hint, .ref .vox-recon__body')) {
    if (!visible(note)) continue;
    const rn = vrect(note);
    for (const v of document.querySelectorAll('.ref .vox-num, .ref .vox-trailing__state-value')) {
      if (!visible(v) || note.contains(v)) continue;
      const rv = vrect(v);
      const ox = Math.min(rn.right, rv.right) - Math.max(rn.left, rv.left);
      const oy = Math.min(rn.bottom, rv.bottom) - Math.max(rn.top, rv.top);
      if (ox > 2 && oy > 2) add('copy-over-values', note, 'overlaps ' + cls(v));
    }
  }

  const h = sel => { const el = document.querySelector(sel); return el ? Math.round(el.getBoundingClientRect().height) : null; };
  const ticket = document.querySelector('.vox-ticket');
  return {
    findings,
    pageOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
    control: h('.vox-btn'), row: h('.vox-table__row'), widgetHeader: h('.vox-widget__header'),
    ticketWidth: ticket ? Math.round(ticket.getBoundingClientRect().width) : null,
    sections: document.querySelectorAll('section.ref__sec').length,
  };
}
"""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--reference", default="frontend/design-system/reference/index.html")
    ap.add_argument("--report", default="design-verify-report.json")
    ap.add_argument("--shots", default="")
    args = ap.parse_args()

    ref = pathlib.Path(args.reference).resolve()
    if not ref.is_file():
        print("reference not found: %s" % ref, file=sys.stderr)
        return 2
    url = ref.as_uri()
    report = {"reference": str(ref), "matrix": {}, "negativeControls": {}}
    failures = []

    if args.shots:
        os.makedirs(args.shots, exist_ok=True)

    with sync_playwright() as p:
        browser = p.chromium.launch()
        for density in DENSITIES:
            ctl, row, wh = GEOMETRY[density]
            for width in WIDTHS:
                page = browser.new_page(viewport={"width": width, "height": 1000})
                page.goto(url)
                page.evaluate("d => document.body.setAttribute('data-density', d)", density)
                page.wait_for_timeout(250)
                r = page.evaluate(PROBE)
                key = "%s-%d" % (density, width)
                report["matrix"][key] = r

                for f in r["findings"]:
                    failures.append("%s: %s in %s (%s) %s" % (key, f["kind"], f["sec"], f["cls"], f["detail"]))
                if r["pageOverflow"] > 0:
                    failures.append("%s: page scrolls horizontally by %dpx" % (key, r["pageOverflow"]))
                for name, got, want in (("control", r["control"], ctl), ("row", r["row"], row),
                                        ("widget header", r["widgetHeader"], wh)):
                    if got != want:
                        failures.append("%s: %s height %s, expected %s" % (key, name, got, want))
                if r["ticketWidth"] is not None and r["ticketWidth"] < 300:
                    failures.append("%s: order ticket %dpx is under its 300px minimum" % (key, r["ticketWidth"]))
                if r["sections"] < 1:
                    failures.append("%s: the reference rendered no sections" % key)

                if args.shots:
                    page.screenshot(path=os.path.join(args.shots, "%s.png" % key), full_page=False)
                page.close()

        # Negative controls: the verifier has to fail when a known defect is present.
        for label, css, expect in NEGATIVE_CONTROLS:
            page = browser.new_page(viewport={"width": 1280, "height": 1000})
            page.goto(url)
            page.add_style_tag(content=css)
            page.wait_for_timeout(250)
            kinds = {}
            for f in page.evaluate(PROBE)["findings"]:
                kinds[f["kind"]] = kinds.get(f["kind"], 0) + 1
            report["negativeControls"][label] = {"expected": expect, "detected": kinds}
            if expect not in kinds:
                failures.append("negative control not detected: %s (expected %s, got %s)" % (label, expect, kinds))
            page.close()
        browser.close()

    report["failures"] = failures
    pathlib.Path(args.report).write_text(json.dumps(report, ensure_ascii=False, indent=1), encoding="utf-8")

    combos = len(report["matrix"])
    print("combinations verified: %d" % combos)
    for label, data in report["negativeControls"].items():
        print("  negative control %-42s -> %s" % (label, data["detected"] or "NOTHING DETECTED"))
    if failures:
        print("\nFAILURES (%d):" % len(failures))
        for f in failures[:60]:
            print("  -", f)
        return 1
    print("\nclean: 0 findings across %d combinations, all %d negative controls detected"
          % (combos, len(NEGATIVE_CONTROLS)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
