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
