/* TauriMusic 宣传页交互：双设计、深浅色与播放页演示，原生 JS 无依赖。 */
(function () {
  "use strict";

  var root = document.documentElement;
  var themeBtn = document.getElementById("themeBtn");
  var themeMeta = document.querySelector('meta[name="theme-color"]');
  var reducedMotion = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  function updateThemeMeta() {
    if (!themeMeta) return;
    var design = root.getAttribute("data-design");
    var theme = root.getAttribute("data-theme");
    themeMeta.setAttribute("content", design === "paper" ? "#f0eee6" : theme === "light" ? "#eef0f7" : "#0b0d14");
  }

  function applyDesign(design) {
    root.setAttribute("data-design", design);
    document.querySelectorAll("[data-design-btn]").forEach(function (btn) {
      btn.setAttribute("aria-pressed", String(btn.getAttribute("data-design-btn") === design));
    });
    updateThemeMeta();
  }

  /* 可通过 ?design=paper 分享指定风格；无参数时恢复上次选择。 */
  var initialDesign = "glass";
  try {
    var queryDesign = new URLSearchParams(window.location.search).get("design");
    var savedDesign = localStorage.getItem("tm2-design");
    if (queryDesign === "paper" || queryDesign === "glass") initialDesign = queryDesign;
    else if (savedDesign === "paper" || savedDesign === "glass") initialDesign = savedDesign;
    var savedTheme = localStorage.getItem("tm2-theme");
    if (savedTheme === "light" || savedTheme === "dark") root.setAttribute("data-theme", savedTheme);
  } catch (e) { /* file:// 或隐私模式下不可存储时保留默认值。 */ }
  applyDesign(initialDesign);

  document.querySelectorAll("[data-design-btn]").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var next = btn.getAttribute("data-design-btn");
      applyDesign(next);
      try { localStorage.setItem("tm2-design", next); } catch (e) { /* 仅失去偏好记忆，不影响切换。 */ }
    });
  });

  if (themeBtn) {
    themeBtn.addEventListener("click", function () {
      var next = root.getAttribute("data-theme") === "dark" ? "light" : "dark";
      root.setAttribute("data-theme", next);
      updateThemeMeta();
      try { localStorage.setItem("tm2-theme", next); } catch (e) { /* 同上。 */ }
    });
  }

  /* 滚动入场；不支持观察器或偏好减少动态时直接显示，避免空白页。 */
  var revealEls = document.querySelectorAll(".reveal");
  function sweep() {
    var vh = window.innerHeight || 800;
    revealEls.forEach(function (el) {
      var r = el.getBoundingClientRect();
      if (r.top < vh * 0.97 && r.bottom > 0) el.classList.add("in");
    });
  }
  if (reducedMotion || !("IntersectionObserver" in window)) {
    revealEls.forEach(function (el) { el.classList.add("in"); });
  } else {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (en.isIntersecting) {
          en.target.classList.add("in");
          io.unobserve(en.target);
        }
      });
    }, { threshold: 0.1 });
    revealEls.forEach(function (el) { io.observe(el); });
    window.addEventListener("load", function () {
      sweep();
      window.setTimeout(sweep, 350);
    });
    sweep();
  }

  /* 玻璃高光只在玻璃设计下计算，纸感设计不付出无谓的指针渲染成本。 */
  var raf = 0;
  document.addEventListener("pointermove", function (e) {
    if (raf || root.getAttribute("data-design") !== "glass") return;
    raf = requestAnimationFrame(function () {
      raf = 0;
      var el = e.target && e.target.closest ? e.target.closest(".glass, .glass-heavy") : null;
      if (!el) return;
      var r = el.getBoundingClientRect();
      el.style.setProperty("--mx", ((e.clientX - r.left) / r.width) * 100 + "%");
      el.style.setProperty("--my", ((e.clientY - r.top) / r.height) * 100 + "%");
    });
  }, { passive: true });

  /* 数字只承担信息层级，不阻塞阅读。 */
  function animateCount(el) {
    var target = parseInt(el.getAttribute("data-count"), 10);
    if (isNaN(target)) return;
    var done = false;
    function finish() {
      if (done) return;
      done = true;
      el.textContent = target.toLocaleString("en-US");
    }
    if (reducedMotion) {
      finish();
      return;
    }
    var t0 = null;
    var dur = 1200;
    function step(ts) {
      if (done) return;
      if (!t0) t0 = ts;
      var p = Math.min(1, (ts - t0) / dur);
      var eased = 1 - Math.pow(1 - p, 3);
      el.textContent = Math.round(target * eased).toLocaleString("en-US");
      if (p < 1) requestAnimationFrame(step);
      else finish();
    }
    requestAnimationFrame(step);
    window.setTimeout(finish, dur + 400);
  }
  var counters = document.querySelectorAll("[data-count]");
  if ("IntersectionObserver" in window && !reducedMotion) {
    var cio = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (en.isIntersecting) {
          animateCount(en.target);
          cio.unobserve(en.target);
        }
      });
    }, { threshold: 0.4 });
    counters.forEach(function (el) { cio.observe(el); });
  } else {
    counters.forEach(animateCount);
  }

  /* 播放页是纯交互示意：不加载音频，不发起网络请求。 */
  var DURATION = 269;
  var playBtn = document.getElementById("npPlay");
  var fill = document.getElementById("npFill");
  var cur = document.getElementById("npCur");
  var track = document.getElementById("npTrack");
  var lyricsEl = document.getElementById("npLyrics");
  if (playBtn && fill && cur && lyricsEl) {
    var lines = Array.prototype.slice.call(lyricsEl.querySelectorAll("li"));
    var pos = 0;
    var playing = false;
    var lastTs = 0;

    function fmt(s) {
      s = Math.max(0, Math.floor(s));
      return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0");
    }
    function render() {
      fill.style.width = (pos / DURATION) * 100 + "%";
      cur.textContent = fmt(pos);
      if (track) {
        track.setAttribute("aria-valuenow", String(Math.floor(pos)));
        track.setAttribute("aria-valuetext", fmt(pos));
      }
      var per = DURATION / lines.length;
      var active = Math.min(lines.length - 1, Math.floor(pos / per));
      lines.forEach(function (li, i) {
        li.classList.toggle("on", i === active);
        li.classList.toggle("done", i < active);
        li.setAttribute("aria-current", i === active ? "true" : "false");
      });
      var li = lines[active];
      if (li) lyricsEl.scrollTop = Math.max(0, li.offsetTop - lyricsEl.clientHeight / 2 + li.clientHeight);
    }
    function tick(ts) {
      if (!playing) return;
      if (!lastTs) lastTs = ts;
      var dt = (ts - lastTs) / 1000;
      lastTs = ts;
      pos += dt * 8;
      if (pos >= DURATION) pos = 0;
      render();
      requestAnimationFrame(tick);
    }
    playBtn.addEventListener("click", function () {
      playing = !playing;
      playBtn.classList.toggle("playing", playing);
      playBtn.setAttribute("aria-pressed", String(playing));
      lastTs = 0;
      if (playing) requestAnimationFrame(tick);
    });
    if (track) {
      track.addEventListener("click", function (e) {
        var r = track.getBoundingClientRect();
        pos = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)) * DURATION;
        render();
      });
      track.addEventListener("keydown", function (e) {
        if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
          pos = Math.max(0, Math.min(DURATION, pos + (e.key === "ArrowRight" ? 5 : -5)));
        } else if (e.key === "Home") pos = 0;
        else if (e.key === "End") pos = DURATION - 0.01;
        else return;
        e.preventDefault();
        render();
      });
    }
    lines.forEach(function (li, i) {
      function seek() {
        pos = (DURATION / lines.length) * i + 0.01;
        render();
      }
      li.addEventListener("click", seek);
      li.addEventListener("keydown", function (e) {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          seek();
        }
      });
    });
    render();
  }
})();
