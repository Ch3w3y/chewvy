// Chewvy site. No dependencies. Every block degrades to the static page if it fails.
(() => {
  'use strict';

  const REPO = 'Ch3w3y/chewvy';
  const still = window.matchMedia('(prefers-reduced-motion: reduce)');
  const $ = (selector, root = document) => root.querySelector(selector);
  const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];

  // ---- Reveal on scroll -------------------------------------------------
  const revealed = $$('[data-reveal]');
  if ('IntersectionObserver' in window && !still.matches) {
    const seen = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          entry.target.classList.add('is-in');
          seen.unobserve(entry.target);
        }
      }
    }, { rootMargin: '0px 0px -8% 0px', threshold: 0.05 });
    revealed.forEach((element) => seen.observe(element));
  } else {
    revealed.forEach((element) => element.classList.add('is-in'));
  }

  // ---- Parallax: the photograph drifts, the hero captures slide ----------
  const layers = $$('.backdrop-layer');
  const movers = $$('[data-parallax]');
  let queued = false;
  const drift = () => {
    queued = false;
    const y = window.scrollY;
    const limit = window.innerHeight * 0.05;
    const offset = Math.max(-limit, Math.min(limit, -y * 0.03));
    layers.forEach((layer) => layer.style.setProperty('--drift', `${offset.toFixed(1)}px`));
    movers.forEach((mover) => {
      mover.style.setProperty('--shift', `${(y * Number(mover.dataset.parallax)).toFixed(1)}px`);
    });
  };
  if (!still.matches) {
    window.addEventListener('scroll', () => {
      if (!queued) {
        queued = true;
        requestAnimationFrame(drift);
      }
    }, { passive: true });
  }

  // ---- Copy buttons -------------------------------------------------------
  $$('.copy').forEach((button) => {
    button.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText($(`#${button.dataset.copy}`).textContent);
        button.textContent = 'Copied';
      } catch {
        button.textContent = 'Select it';
      }
      setTimeout(() => { button.textContent = 'Copy'; }, 1600);
    });
  });

  // ---- Downloads: suggest this computer's package, link the newest release
  const platform = (() => {
    const hint = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
    const agent = navigator.userAgent || '';
    if (/Android|iPhone|iPad|iPod/i.test(agent) || (/Mac/i.test(hint) && navigator.maxTouchPoints > 1)) return 'mobile';
    if (/Win/i.test(hint)) return 'windows';
    if (/Mac/i.test(hint)) return 'macos';
    if (/Linux|X11/i.test(hint) || /Linux/i.test(agent)) return 'linux';
    return 'unknown';
  })();
  const primary = $('#dl-primary');
  const primaryLabel = $('#dl-primary-label');
  const note = $('#dl-note');
  const primaryAsset = { windows: 'windows-x86_64-setup.exe', macos: 'macos-arm64.dmg' }[platform];
  $(`.platforms [data-os="${platform}"]`)?.classList.add('is-yours');
  if (platform === 'linux') {
    primaryLabel.textContent = 'Install on Linux';
    primary.href = '#download';
  } else if (platform === 'windows') {
    primaryLabel.textContent = 'Download for Windows';
    primary.dataset.asset = primaryAsset;
  } else if (platform === 'macos') {
    primaryLabel.textContent = 'Download for macOS';
    primary.dataset.asset = primaryAsset;
    note.prepend('Needs Apple Silicon. ');
  } else if (platform === 'mobile') {
    primaryLabel.textContent = 'See the desktop downloads';
    primary.href = '#download';
    note.prepend('Chewvy is a desktop app. ');
  }

  fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: 'application/vnd.github+json' } })
    .then((answer) => (answer.ok ? answer.json() : Promise.reject(new Error(answer.status))))
    .then((release) => {
      const version = String(release.tag_name || '').replace(/^v/, '');
      for (const link of $$('[data-asset]')) {
        const asset = (release.assets || []).find((candidate) => candidate.name.endsWith(link.dataset.asset));
        if (asset) link.href = asset.browser_download_url;
      }
      if (version) $('#dl-version').textContent = `, now at version ${version}`;
    })
    .catch(() => { /* the links already point at the release page */ });
})();
