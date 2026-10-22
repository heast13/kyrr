// Tiny enhancements. The page works without this file.
(function () {
  "use strict";
  var root = document.documentElement;
  root.classList.add("js");

  // Remember an explicit language choice (localStorage only, no cookies).
  document.querySelectorAll("[data-lang-switch]").forEach(function (a) {
    a.addEventListener("click", function () {
      try { localStorage.setItem("kyrr-lang", a.getAttribute("data-lang-switch")); } catch (e) {}
    });
  });

  // Copy buttons on the install commands (the commands stay selectable without this).
  var de = (root.lang || "").slice(0, 2) === "de";
  var txt = de ? { copy: "Kopieren", done: "Kopiert!", fail: "Markieren und Strg+C" }
               : { copy: "Copy", done: "Copied!", fail: "Select and press Ctrl+C" };
  document.querySelectorAll(".codebox").forEach(function (box) {
    var b = document.createElement("button");
    b.type = "button";
    b.className = "copy";
    b.setAttribute("aria-live", "polite");
    b.textContent = txt.copy;
    b.addEventListener("click", function () {
      var code = box.querySelector("code").textContent;
      var ok = function () { b.textContent = txt.done; setTimeout(function () { b.textContent = txt.copy; }, 2000); };
      var fail = function () { b.textContent = txt.fail; setTimeout(function () { b.textContent = txt.copy; }, 3000); };
      if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(code).then(ok, fail);
      else fail();
    });
    box.appendChild(b);
  });

  // The remark under the main heading swaps to another one every twelve seconds.
  // Not with reduced motion, and not while the tab is in the background.
  var quip = document.querySelector("[data-quips]");
  var still = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (quip && !still) {
    var line = quip.parentNode;
    var words = quip.getAttribute("data-quips").split("|");
    var at = 0;
    setInterval(function () {
      if (document.hidden) return;
      line.classList.add("out");
      setTimeout(function () {
        at = (at + 1) % words.length;
        quip.textContent = words[at];
        line.classList.remove("out");
      }, 450);
    }, 12000);
  }

  // Install tabs: built from the panels, so without JS they simply stack.
  var wrap = document.querySelector("[data-tabs]");
  if (!wrap) return;
  var panels = Array.prototype.slice.call(wrap.querySelectorAll(".panel"));
  var list = document.createElement("div");
  list.className = "tablist";
  list.setAttribute("role", "tablist");
  var tabs = panels.map(function (p, i) {
    var b = document.createElement("button");
    b.type = "button";
    b.setAttribute("role", "tab");
    b.id = "tab-" + p.id;
    b.setAttribute("aria-controls", p.id);
    b.textContent = p.getAttribute("data-label");
    b.addEventListener("click", function () { select(i, false); });
    p.setAttribute("role", "tabpanel");
    p.setAttribute("aria-labelledby", b.id);
    p.tabIndex = 0;
    list.appendChild(b);
    return b;
  });
  list.addEventListener("keydown", function (e) {
    var i = tabs.indexOf(document.activeElement);
    if (i < 0) return;
    var n = tabs.length, j = null;
    if (e.key === "ArrowRight") j = (i + 1) % n;
    else if (e.key === "ArrowLeft") j = (i + n - 1) % n;
    else if (e.key === "Home") j = 0;
    else if (e.key === "End") j = n - 1;
    if (j !== null) { e.preventDefault(); select(j, true); }
  });
  function select(i, focus) {
    tabs.forEach(function (t, k) {
      t.setAttribute("aria-selected", k === i ? "true" : "false");
      t.tabIndex = k === i ? 0 : -1;
      panels[k].hidden = k !== i;
    });
    if (focus) tabs[i].focus();
  }
  wrap.insertBefore(list, wrap.firstChild);

  // Start on the visitor's system: Windows, else Linux (the AppImage).
  var ua = navigator.userAgent || "";
  var want = /Windows/.test(ua) ? "windows" : "appimage";
  var start = panels.findIndex(function (p) { return p.id === want; });
  var hash = panels.findIndex(function (p) { return "#" + p.id === location.hash; });
  select(hash >= 0 ? hash : Math.max(start, 0), false);
})();

// The dictation demo: hold the key, the display shows the words as they are heard, and
// after the key is let go the sentence is written into the notes window. One saying after
// another in random order, never the same one twice in a row. A click or tap pauses and
// resumes it; it also waits while scrolled out of view. Visitors who asked for less motion
// get one saying standing still with a play button.
(function () {
  var screen = document.querySelector("[data-demo]");
  if (!screen) return;
  var video = screen.querySelector("video");
  var doc = screen.querySelector(".doc");
  var caret = doc.querySelector(".caret");
  var key = document.querySelector(".demo kbd");
  var base = screen.getAttribute("data-base");
  var lang = screen.getAttribute("data-lang");
  var said = screen.getAttribute("data-sayings").split("|");
  var still = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var dark = window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)");
  // The display shows its words until 1.8 s before the end of a clip, then writes for a
  // second; the sentence lands in the document when the display goes away.
  var HOLD_END = 1.8, WRITE_END = 0.8, GAP = 4;
  var order = [];
  var last = -1;
  var phase = "idle";
  var gap = 0;
  var inserted = false;
  var userPaused = false;
  var visible = true;
  var spans = [];

  function theme() {
    var t = document.documentElement.getAttribute("data-theme");
    if (t === "dark" || t === "light") return t;
    return dark && dark.matches ? "dark" : "light";
  }
  function pick() {
    if (!order.length) {
      for (var i = 0; i < said.length; i++) order.push(i);
      for (var j = order.length - 1; j > 0; j--) {
        var k = Math.floor(Math.random() * (j + 1));
        var tmp = order[j]; order[j] = order[k]; order[k] = tmp;
      }
      // The new round does not open with the saying that just ended.
      if (order[0] === last && said.length > 1) order.push(order.shift());
    }
    last = order.shift();
    return last;
  }
  // The caret sits behind the last word shown, or in front of the first one.
  function place(at, after) {
    var r = at.getBoundingClientRect();
    var d = doc.getBoundingClientRect();
    caret.style.left = (after ? r.right : r.left) - d.left + "px";
    caret.style.top = r.top - d.top + (r.height - caret.offsetHeight) / 2 + "px";
  }
  function line(text, shown) {
    var p = document.createElement("p");
    spans = text.split(" ").map(function (w, i) {
      var s = document.createElement("span");
      s.className = shown ? "word" : "word hide";
      s.textContent = (i ? " " : "") + w;
      p.appendChild(s);
      return s;
    });
    doc.insertBefore(p, caret);
    return p;
  }
  // A new empty line in the document; when the window is full it is cleared first.
  function open(text, ready) {
    var p = line(text, false);
    if (doc.scrollHeight <= doc.clientHeight + 1) { place(spans[0], false); return ready(); }
    doc.classList.add("fade");
    setTimeout(function () {
      while (doc.firstChild !== caret) doc.removeChild(doc.firstChild);
      line(text, false);
      place(spans[0], false);
      doc.classList.remove("fade");
      ready();
    }, 450);
  }
  function reveal() {
    var mine = spans;
    caret.classList.add("typing");
    mine.forEach(function (s, i) {
      setTimeout(function () {
        s.classList.remove("hide");
        place(s, true);
        if (i === mine.length - 1) caret.classList.remove("typing");
      }, i * 35);
    });
  }
  function sync() {
    if (phase !== "video") return;
    if (userPaused || !visible) video.pause();
    else video.play().catch(function () {});
  }
  function begin() {
    var n = pick();
    phase = "idle";
    open(said[n], function () {
      inserted = false;
      phase = "video";
      video.src = base + lang + "-" + theme() + "-" + (n + 1) + ".webm";
      sync();
    });
  }
  var before = 0;
  function tick(now) {
    var dt = Math.min(now - before, 100) / 1000;
    before = now;
    if (phase === "video" && video.duration && !video.paused) {
      var t = video.currentTime, d = video.duration;
      key.classList.toggle("down", t > 0.1 && t < d - HOLD_END);
      if (!inserted && t >= d - WRITE_END) { inserted = true; reveal(); }
    } else if (phase === "gap" && !userPaused && visible) {
      gap += dt;
      if (gap >= GAP) begin();
    }
    requestAnimationFrame(tick);
  }

  function start() {
    // Click, tap or Enter/Space pauses and resumes.
    function toggle() {
      userPaused = !userPaused;
      sync();
    }
    screen.classList.add("play");
    screen.tabIndex = 0;
    screen.addEventListener("click", toggle);
    screen.addEventListener("keydown", function (e) {
      if (e.key === "Enter" || e.key === " ") { e.preventDefault(); toggle(); }
    });

    // A clip that cannot play is skipped, so the demo never stands still on a failure.
    function done() {
      key.classList.remove("down");
      if (!inserted) { inserted = true; reveal(); }
      gap = 0;
      phase = "gap";
    }
    video.addEventListener("ended", done);
    video.addEventListener("error", function () { if (phase === "video") done(); });
    if (window.IntersectionObserver) {
      new IntersectionObserver(function (e) {
        visible = e[e.length - 1].isIntersecting;
        sync();
      }).observe(screen);
    }
    // Without script the document shows the first saying as plain text; here it starts empty.
    while (doc.firstChild !== caret) doc.removeChild(doc.firstChild);
    begin();
    requestAnimationFrame(tick);
  }

  if (!still) return start();

  // One saying, written into the document. The visitor
  // asked for less motion, so the demo only moves after a click on the play button.
  var n = pick();
  while (doc.firstChild !== caret) doc.removeChild(doc.firstChild);
  line(said[n], true);
  place(spans[spans.length - 1], true);
  caret.classList.add("typing");
  key.classList.add("down");
  var go = document.createElement("button");
  go.type = "button";
  go.className = "go";
  go.textContent = lang === "de" ? "Abspielen" : "Play";
  go.addEventListener("click", function (e) {
    e.stopPropagation();
    go.remove();
    key.classList.remove("down");
    start();
  });
  screen.appendChild(go);
})();
