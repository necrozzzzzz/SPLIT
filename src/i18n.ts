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
  en: { settings: 'Settings', general: 'General', language: 'Language', auto: 'Automatic (Windows language)', preferences: 'Preferences, shortcuts and diagnostics', chooseLanguage: 'Choose the language used in SPLIT.', favorites: "Favorites", savestates: "Savestates", refresh: "Refresh", checking: "Checking…", undo: "Undo", redo: "Redo", connected: "Connected", notConnected: "Not connected", meaning: "What it means", troubleshooting: "If something is wrong", whatDoesItMean: "What does this mean?", systemHealth: "System health", replayGuide: "Replay getting started guide" },
  fr: { settings: 'Paramètres', general: 'Général', language: 'Langue', auto: 'Automatique (langue de Windows)', preferences: 'Préférences, raccourcis et diagnostics', chooseLanguage: 'Choisissez la langue utilisée dans SPLIT.', favorites: "Favoris", savestates: "Sauvegardes", refresh: "Actualiser", checking: "Vérification…", undo: "Annuler", redo: "Rétablir", connected: "Connecté", notConnected: "Non connecté", meaning: "À quoi ça sert", troubleshooting: "En cas de problème", whatDoesItMean: "Que signifie cet indicateur ?", systemHealth: "État du système", replayGuide: "Revoir le tutoriel de démarrage" },
  es: { settings: 'Configuración', general: 'General', language: 'Idioma', auto: 'Automático (idioma de Windows)', preferences: 'Preferencias, atajos y diagnósticos', chooseLanguage: 'Elige el idioma de SPLIT.', favorites: "Favoritos", savestates: "Partidas guardadas", refresh: "Actualizar", checking: "Comprobando…", undo: "Deshacer", redo: "Rehacer", connected: "Conectado", notConnected: "Desconectado", meaning: "Qué significa", troubleshooting: "Si hay un problema", whatDoesItMean: "¿Qué significa esto?", systemHealth: "Estado del sistema", replayGuide: "Repetir guía de inicio" },
  de: { settings: 'Einstellungen', general: 'Allgemein', language: 'Sprache', auto: 'Automatisch (Windows-Sprache)', preferences: 'Einstellungen, Tastenkürzel und Diagnose', chooseLanguage: 'Wähle die Sprache von SPLIT.', favorites: "Favoriten", savestates: "Spielstände", refresh: "Aktualisieren", checking: "Prüfe…", undo: "Rückgängig", redo: "Wiederholen", connected: "Verbunden", notConnected: "Nicht verbunden", meaning: "Was es bedeutet", troubleshooting: "Bei Problemen", whatDoesItMean: "Was bedeutet das?", systemHealth: "Systemzustand", replayGuide: "Einführung erneut ansehen" },
  pt: { settings: 'Configurações', general: 'Geral', language: 'Idioma', auto: 'Automático (idioma do Windows)', preferences: 'Preferências, atalhos e diagnósticos', chooseLanguage: 'Escolha o idioma do SPLIT.', favorites: "Favoritos", savestates: "Estados salvos", refresh: "Atualizar", checking: "Verificando…", undo: "Desfazer", redo: "Refazer", connected: "Conectado", notConnected: "Desconectado", meaning: "O que significa", troubleshooting: "Se houver um problema", whatDoesItMean: "O que isso significa?", systemHealth: "Estado do sistema", replayGuide: "Rever guia inicial" },
  ru: { settings: 'Настройки', general: 'Общие', language: 'Язык', auto: 'Автоматически (язык Windows)', preferences: 'Параметры, горячие клавиши и диагностика', chooseLanguage: 'Выберите язык SPLIT.', favorites: "Избранное", savestates: "Сохранения", refresh: "Обновить", checking: "Проверка…", undo: "Отменить", redo: "Повторить", connected: "Подключено", notConnected: "Не подключено", meaning: "Что это значит", troubleshooting: "Если возникла проблема", whatDoesItMean: "Что это значит?", systemHealth: "Состояние системы", replayGuide: "Повторить руководство" },
  ja: { settings: '設定', general: '一般', language: '言語', auto: '自動（Windows の言語）', preferences: '設定、ショートカット、診断', chooseLanguage: 'SPLIT の言語を選択してください。', favorites: "お気に入り", savestates: "セーブデータ", refresh: "更新", checking: "確認中…", undo: "元に戻す", redo: "やり直す", connected: "接続済み", notConnected: "未接続", meaning: "意味", troubleshooting: "問題が発生した場合", whatDoesItMean: "これはどういう意味ですか？", systemHealth: "システムの状態", replayGuide: "ガイドをもう一度見る" },
  zh: { settings: '设置', general: '常规', language: '语言', auto: '自动（Windows 语言）', preferences: '首选项、快捷键和诊断', chooseLanguage: '选择 SPLIT 使用的语言。', favorites: "收藏", savestates: "存档", refresh: "刷新", checking: "检查中…", undo: "撤销", redo: "重做", connected: "已连接", notConnected: "未连接", meaning: "含义", troubleshooting: "如果出现问题", whatDoesItMean: "这是什么意思？", systemHealth: "系统状态", replayGuide: "重看入门指南" },
} as const;
export type MessageKey = keyof typeof messages.en;
export function t(key: MessageKey, language = getLanguage()): string {
  return messages[language][key];
}
