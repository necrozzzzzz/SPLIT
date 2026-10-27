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

export const messages = {
  en: { settings: 'Settings', general: 'General', language: 'Language', auto: 'Automatic (Windows language)', preferences: 'Preferences, shortcuts and diagnostics', chooseLanguage: 'Choose the language used in SPLIT.' },
  fr: { settings: 'Paramètres', general: 'Général', language: 'Langue', auto: 'Automatique (langue de Windows)', preferences: 'Préférences, raccourcis et diagnostics', chooseLanguage: 'Choisissez la langue utilisée dans SPLIT.' },
  es: { settings: 'Configuración', general: 'General', language: 'Idioma', auto: 'Automático (idioma de Windows)', preferences: 'Preferencias, atajos y diagnósticos', chooseLanguage: 'Elige el idioma de SPLIT.' },
  de: { settings: 'Einstellungen', general: 'Allgemein', language: 'Sprache', auto: 'Automatisch (Windows-Sprache)', preferences: 'Einstellungen, Tastenkürzel und Diagnose', chooseLanguage: 'Wähle die Sprache von SPLIT.' },
  pt: { settings: 'Configurações', general: 'Geral', language: 'Idioma', auto: 'Automático (idioma do Windows)', preferences: 'Preferências, atalhos e diagnósticos', chooseLanguage: 'Escolha o idioma do SPLIT.' },
  ru: { settings: 'Настройки', general: 'Общие', language: 'Язык', auto: 'Автоматически (язык Windows)', preferences: 'Параметры, горячие клавиши и диагностика', chooseLanguage: 'Выберите язык SPLIT.' },
  ja: { settings: '設定', general: '一般', language: '言語', auto: '自動（Windows の言語）', preferences: '設定、ショートカット、診断', chooseLanguage: 'SPLIT の言語を選択してください。' },
  zh: { settings: '设置', general: '常规', language: '语言', auto: '自动（Windows 语言）', preferences: '首选项、快捷键和诊断', chooseLanguage: '选择 SPLIT 使用的语言。' },
} as const;
export type MessageKey = keyof typeof messages.en;
export function t(key: MessageKey, language = getLanguage()): string {
  return messages[language][key];
}
