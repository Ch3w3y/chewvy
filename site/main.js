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

  // ---- Backdrops: swap the photograph the way the app's themes do -------
  // The same windows and seasons as the app (theme_art/calendar.rs).
  const SOUTH = /^(Australia|Antarctica)\/|^Pacific\/(Auckland|Chatham|Fiji)|^Africa\/(Johannesburg|Maputo|Windhoek|Harare|Gaborone|Lusaka)|^America\/(Sao_Paulo|Argentina|Santiago|Montevideo|Asuncion|La_Paz|Lima)/;
  const seasonalKey = (now) => {
    const month = now.getMonth() + 1;
    const day = now.getDate();
    if ((month === 10 && day >= 15) || (month === 11 && day === 1)) return 'halloween';
    if (month === 12 || (month === 1 && day <= 6)) return 'christmas';
    const north = month === 12 || month <= 2 ? 'winter' : month <= 5 ? 'spring' : month <= 8 ? 'summer' : 'autumn';
    let zone = '';
    try { zone = Intl.DateTimeFormat().resolvedOptions().timeZone || ''; } catch { /* keep the northern season */ }
    return SOUTH.test(zone) ? { winter: 'summer', spring: 'autumn', summer: 'winter', autumn: 'spring' }[north] : north;
  };

  const groupsHost = $('#theme-groups');
  const creditLine = $('#theme-credit');
  const image = (stem, thumb) => `assets/themes/${stem}${thumb ? '-thumb' : ''}.webp`;
  let shownLayer = 0;
  let request = 0;

  const showPhoto = (theme, index) => {
    const photo = theme.photos[index % theme.photos.length];
    const mine = ++request;
    const loader = new Image();
    loader.onload = () => {
      if (mine !== request) return;
      const next = layers[1 - shownLayer];
      next.style.backgroundImage = `url('${image(photo.image)}')`;
      next.classList.add('is-on');
      layers[shownLayer].classList.remove('is-on');
      shownLayer = 1 - shownLayer;
    };
    loader.src = image(photo.image);
    creditLine.textContent = '';
    const source = Object.assign(document.createElement('a'), { href: photo.source, textContent: photo.title });
    creditLine.append(`Showing ${theme.name}: `, source, ` by ${photo.author}, ${photo.licence}.`);
    if (theme.photos.length > 1) creditLine.append(' Choose it again for its other photograph.');
  };

  const buildThemes = (themes) => {
    const byKey = new Map(themes.map((theme) => [theme.key, theme]));
    const buttons = [];
    const press = (button) => buttons.forEach((other) => other.setAttribute('aria-pressed', String(other === button)));
    const sections = [
      ['Default', themes.filter((theme) => theme.group === 'default'), true],
      ['Genres', themes.filter((theme) => theme.group === 'genre'), false],
      ['Festivals and seasons', themes.filter((theme) => theme.group === 'festival' || theme.group === 'season'), false],
    ];
    for (const [title, members, withAutomatic] of sections) {
      const group = document.createElement('div');
      group.className = 'theme-group';
      const heading = Object.assign(document.createElement('h3'), { textContent: title });
      const list = document.createElement('div');
      list.className = 'theme-list';
      for (const theme of members) {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'theme';
        button.style.backgroundImage = `url('${image(theme.photos[0].image, true)}')`;
        button.setAttribute('aria-pressed', String(theme.key === 'aurora'));
        button.append(Object.assign(document.createElement('span'), { textContent: theme.name }));
        let turn = 0;
        button.addEventListener('click', () => {
          turn = button.getAttribute('aria-pressed') === 'true' ? turn + 1 : 0;
          press(button);
          showPhoto(theme, turn);
        });
        buttons.push(button);
        list.append(button);
      }
      if (withAutomatic) {
        const seasonal = byKey.get(seasonalKey(new Date()));
        if (seasonal) {
          const auto = document.createElement('button');
          auto.type = 'button';
          auto.className = 'theme theme-auto';
          auto.setAttribute('aria-pressed', 'false');
          auto.append(Object.assign(document.createElement('span'), { textContent: `Automatic · ${seasonal.name}` }));
          auto.addEventListener('click', () => { press(auto); showPhoto(seasonal, 0); });
          buttons.push(auto);
          list.append(auto);
        }
      }
      group.append(heading, list);
      groupsHost.append(group);
    }
    const aurora = byKey.get('aurora');
    if (aurora) {
      const photo = aurora.photos[0];
      const source = Object.assign(document.createElement('a'), { href: photo.source, textContent: photo.title });
      creditLine.append('Showing Aurora: ', source, ` by ${photo.author}, ${photo.licence}.`);
    }
  };

  if (groupsHost) {
    fetch('assets/themes/themes.json')
      .then((answer) => (answer.ok ? answer.json() : Promise.reject(new Error(answer.status))))
      .then(buildThemes)
      .catch(() => { groupsHost.textContent = 'The backdrop list could not be loaded.'; });
  }

  // ---- Screens: tabs over one framed capture -----------------------------
  const tabs = $$('.tabs [role="tab"]');
  const shot = $('#shot-img');
  const shotLink = $('#shot-link');
  const caption = $('#shot-caption');
  const panel = $('#shot-panel');
  const files = (name) => ({
    src: `assets/shots/${name}-1600.webp`,
    srcset: `assets/shots/${name}-960.webp 960w, assets/shots/${name}-1600.webp 1600w`,
    full: `assets/shots/${name}.webp`,
  });
  const select = (tab, focus) => {
    tabs.forEach((other) => {
      other.setAttribute('aria-selected', String(other === tab));
      other.tabIndex = other === tab ? 0 : -1;
    });
    if (focus) tab.focus();
    tab.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    panel.setAttribute('aria-labelledby', tab.id);
    const next = files(tab.dataset.shot);
    const apply = () => {
      shot.srcset = next.srcset;
      shot.src = next.src;
      shot.alt = tab.dataset.alt;
      shotLink.href = next.full;
      caption.textContent = tab.dataset.caption;
      shot.classList.remove('is-swapping');
    };
    if (still.matches) {
      apply();
      return;
    }
    shot.classList.add('is-swapping');
    const loader = new Image();
    loader.onload = loader.onerror = () => setTimeout(apply, 120);
    loader.srcset = next.srcset;
    loader.sizes = shot.sizes;
    loader.src = next.src;
  };
  const step = (by, focus) => {
    const current = tabs.findIndex((tab) => tab.getAttribute('aria-selected') === 'true');
    select(tabs[(current + by + tabs.length) % tabs.length], focus);
  };
  tabs.forEach((tab) => {
    tab.addEventListener('click', () => select(tab, false));
    tab.addEventListener('keydown', (event) => {
      if (event.key === 'ArrowRight') { event.preventDefault(); step(1, true); }
      if (event.key === 'ArrowLeft') { event.preventDefault(); step(-1, true); }
      if (event.key === 'Home') { event.preventDefault(); select(tabs[0], true); }
      if (event.key === 'End') { event.preventDefault(); select(tabs[tabs.length - 1], true); }
    });
  });
  $('#shot-prev')?.addEventListener('click', () => step(-1, false));
  $('#shot-next')?.addEventListener('click', () => step(1, false));

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
    note.prepend('The macOS build is experimental and needs Apple Silicon. ');
  } else if (platform === 'mobile') {
    primaryLabel.textContent = 'See the desktop downloads';
    primary.href = '#download';
    note.prepend('Chewvy is a desktop app: there is no phone or tablet version. ');
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
