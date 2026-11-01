// The settings window's search: whether a row matches what's typed. Each word typed must begin a word of the row's
// text, its section's name or one of its synonyms, in any order ("transp" finds opacity, "do not disturb" finds focus).
// Loaded by settings.html; node --test test/settings-search.test.cjs runs it too.
(function (root) {
  const SYN = {
    on_top: 'pin float above front over windows',
    size: 'scale zoom big bigger small smaller resize large medium width',
    layout: 'dock ring pill corners strip hud position',
    skin: 'look style design appearance hud scouter capsule corp drawer classic',
    theme: 'dark mode light auto appearance night colour color',
    ki_buttons: 'ki kaioken instant transmission meter buttons cooldown recharge fill look',
    hud: 'interface overlay buttons power level show',
    fps: 'frame rate fps smooth performance cpu speed animation',
    battery_auto: 'power battery unplugged laptop energy low power mode saver',
    mode: 'floating window card frame background',
    ghost: 'click through clickthrough mouse ignore clicks input passthrough',
    auto_hide: 'fullscreen full screen presentation slideshow games hide',
    clouds: 'weather sky',
    cloud_shape: 'weather sky puffs',
    cloud_amount: 'weather sky more fewer',
    opacity: 'transparent transparency see through translucent fade alpha visibility dim',
    shortcut: 'hotkey keyboard key binding keys show hide toggle',
    menubar_power: 'tray title number menu bar status',
    tray_icon: 'tray colour color theme light dark icon',
    login: 'startup start autostart boot launch',
    update_check: 'updates upgrade version new release',
    sound: 'audio volume mute sfx music noise',
    recap: 'away back idle summary welcome return while you were away',
    raid_windows: 'spectacle villain raid desktop windows overlay boss fun effects',
    screen_beam: 'spectacle kamehameha galick final flash beam screen desktop overlay fun effects',
    profile: 'preset presets profiles work play mode save load switch',
    notify: 'notifications alerts banners popups',
    permission: 'notifications alerts allow permission blocked',
    quiet: 'quiet hours night sleep schedule silence mute do not disturb dnd',
    follow_focus: 'focus do not disturb dnd focus assist silence mute quiet banners',
    snooze: 'snooze pause mute silence later resume quiet do not disturb dnd',
    settings_style: 'settings window compact full wide preview tabs style',
    tray_style: 'menu bar tray popover click icon scouter quick',
    intro: 'intro onboarding welcome tutorial tips first run help again',
  };
  const words = s => String(s || '').toLowerCase().normalize('NFKD').replace(/[̀-ͯ]/g, '').split(/[^a-z0-9]+/).filter(Boolean);
  // `text`: the row's own words; `key`: its setting (for synonyms); `section`: the heading it's under
  function matches(query, text, key, section) {
    const q = words(query);
    if (!q.length) return true;
    const hay = words([text, section, SYN[key]].join(' '));
    return q.every(w => hay.some(h => h.startsWith(w)));
  }
  root.SettingsSearch = { matches, words, SYN };
  if (typeof module === 'object' && module.exports) module.exports = root.SettingsSearch;
})(typeof window === 'object' ? window : globalThis);
