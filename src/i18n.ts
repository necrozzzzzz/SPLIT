export const languages = ['en', 'fr', 'es', 'de', 'pt', 'ru', 'ja', 'zh'] as const;
export type Language = typeof languages[number];
export const languageNames: Record<Language, string> = { en: 'English', fr: 'Français', es: 'Español', de: 'Deutsch', pt: 'Português', ru: 'Русский', ja: '日本語', zh: '简体中文' };
export function getLanguage(): Language {
  const saved = localStorage.getItem('split.language');
  if (saved && languages.includes(saved as Language)) return saved as Language;
  const detected = navigator.language.toLowerCase().split('-')[0] as Language;
  return languages.includes(detected) ? detected : 'en';
}
export function setLanguage(language: Language) {
  localStorage.setItem('split.language', language);
  window.dispatchEvent(new Event('split-language-change'));
}
