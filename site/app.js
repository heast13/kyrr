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

// The dictation display plays its sayings one after another in random order,
// never the same one twice in a row. A click or tap pauses and resumes it.
// Visitors who asked for less motion get one saying standing still, with words shown.
(function () {
  var pill = document.querySelector(".pill-demo video");
  if (!pill) return;
  var base = pill.getAttribute("data-base");
  var lang = pill.getAttribute("data-lang");
  var count = parseInt(pill.getAttribute("data-count"), 10);
  var still = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var dark = window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)");
  var order = [];
  var last = -1;

  function theme() {
    var t = document.documentElement.getAttribute("data-theme");
    if (t === "dark" || t === "light") return t;
    return dark && dark.matches ? "dark" : "light";
  }
  function play(n) {
    last = n;
    pill.src = base + lang + "-" + theme() + "-" + (n + 1) + ".webm";
    pill.play().catch(function () {});
  }
  function next() {
    if (!order.length) {
      for (var i = 0; i < count; i++) order.push(i);
      for (var j = order.length - 1; j > 0; j--) {
        var k = Math.floor(Math.random() * (j + 1));
        var tmp = order[j]; order[j] = order[k]; order[k] = tmp;
      }
      // The new round does not open with the saying that just ended.
      if (order[0] === last && count > 1) order.push(order.shift());
    }
    play(order.shift());
  }

  pill.style.cursor = "pointer";
  pill.addEventListener("click", function () {
    if (pill.paused) pill.play().catch(function () {});
    else pill.pause();
  });
  if (still) {
    play(0);
    pill.pause();
    pill.addEventListener("loadeddata", function () { pill.currentTime = 4; }, { once: true });
    return;
  }
  // A short pause between sayings, so the display does not feel busy.
  pill.addEventListener("ended", function () { setTimeout(next, 4000); });
  next();
})();
