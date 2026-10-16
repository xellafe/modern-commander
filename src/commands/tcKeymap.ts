// Mappa di fabbrica di Total Commander (chord -> id comando).
// Gli id sono i nomi interni di TC (cm_*, verificati sulla lista comandi ufficiale);
// i comandi senza equivalente TC usano il prefisso mc_.
export const DEFAULT_KEYMAP: Record<string, string> = {
  F3: 'cm_List',
  F4: 'cm_Edit',
  F5: 'cm_Copy',
  'Shift+F5': 'cm_CopySamepanel', // in TC il nome è cm_CopySamepanel (p minuscola)
  F6: 'cm_RenMov',
  'Shift+F6': 'cm_RenameOnly',
  F7: 'cm_MkDir',
  F8: 'cm_Delete',
  Delete: 'cm_Delete',
  'Shift+F8': 'mc_DeletePermanent',
  'Shift+Delete': 'mc_DeletePermanent',
  'Alt+F4': 'cm_Exit',

  Tab: 'mc_FocusNextGroup',
  'Shift+Tab': 'mc_FocusPrevGroup',
  Enter: 'mc_Open',
  Backspace: 'cm_GoToParent',
  'Ctrl+PageUp': 'cm_GoToParent',
  'Ctrl+Oem5': 'cm_GoToRoot', // Ctrl+\
  'Alt+Left': 'mc_HistoryBack',
  'Alt+Right': 'mc_HistoryForward',
  'Alt+Down': 'cm_DirectoryHistory',

  Insert: 'mc_MarkAndDown',
  Space: 'mc_MarkOrSize',
  NumAdd: 'cm_SpreadSelection',
  NumSubtract: 'cm_ShrinkSelection',
  NumMultiply: 'cm_ExchangeSelection',
  'Ctrl+A': 'cm_SelectAll',
  'Ctrl+NumAdd': 'cm_SelectAll',
  'Ctrl+NumSubtract': 'cm_ClearAll',

  'Ctrl+T': 'cm_OpenNewTab',
  'Ctrl+W': 'cm_CloseCurrentTab',
  'Ctrl+Tab': 'cm_SwitchToNextTab',
  'Ctrl+Shift+Tab': 'cm_SwitchToPreviousTab',

  'Ctrl+R': 'cm_RereadSource',
  'Ctrl+D': 'cm_DirectoryHotlist',
  'Ctrl+F1': 'cm_SrcShort',
  'Ctrl+F2': 'cm_SrcLong',
  'Ctrl+F3': 'cm_SrcByName',
  'Ctrl+F4': 'cm_SrcByExt',
  'Ctrl+F5': 'cm_SrcByDateTime',
  'Ctrl+F6': 'cm_SrcBySize',
  'Ctrl+F7': 'cm_SrcUnsorted',
  'Ctrl+S': 'mc_QuickFilter',
  'Ctrl+U': 'cm_Exchange',

  'Ctrl+Left': 'mc_OpenInPrevGroup',
  'Ctrl+Right': 'mc_OpenInNextGroup',
  'Alt+F1': 'mc_OpenDrives',
  'Alt+Enter': 'cm_Properties',
  'Shift+F10': 'mc_ContextMenu',
  'Shift+Alt+Enter': 'cm_CountDirContent',

  // Scorciatoie nuove (non presenti nella mappa di fabbrica di TC)
  'Ctrl+Shift+E': 'mc_ToggleSidebar',
  'Ctrl+Oem3': 'mc_ToggleBottomPanel', // Ctrl+ò sul layout italiano
  'Ctrl+Comma': 'mc_Settings',
  'Ctrl+Shift+Oem5': 'mc_SplitGroup', // Ctrl+Shift+\ (come "vai alla parentesi" di VS Code; libero in TC)
};
