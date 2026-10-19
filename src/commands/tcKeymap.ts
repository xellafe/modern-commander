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

  // Cursore (in TC sono tasti fissi, senza id cm_*)
  Up: 'mc_CursorUp',
  Down: 'mc_CursorDown',
  PageUp: 'mc_CursorPageUp',
  PageDown: 'mc_CursorPageDown',
  Home: 'mc_CursorHome',
  End: 'mc_CursorEnd',
  Left: 'mc_CursorLeft',
  Right: 'mc_CursorRight',
  'Shift+Up': 'mc_MarkUp',
  'Shift+Down': 'mc_MarkDown',

  Tab: 'mc_FocusNextGroup',
  'Shift+Tab': 'mc_FocusPrevGroup',
  Enter: 'cm_Return',
  Backspace: 'cm_GoToParent',
  'Ctrl+PageUp': 'cm_GoToParent',
  'Ctrl+Oem5': 'cm_GoToRoot', // Ctrl+\
  'Alt+Left': 'cm_GotoPreviousDir',
  'Alt+Right': 'cm_GotoNextDir',
  'Alt+Down': 'cm_DirectoryHistory',

  Insert: 'cm_Reverse',
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
  'Ctrl+S': 'cm_QuickFilter',
  'Ctrl+U': 'cm_Exchange',

  'Ctrl+Left': 'mc_OpenInPrevGroup',
  'Ctrl+Right': 'mc_OpenInNextGroup',
  'Alt+F1': 'cm_SrcOpenDrives',
  'Alt+Enter': 'cm_Properties',
  'Shift+F10': 'cm_ContextMenu',
  'Shift+Alt+Enter': 'cm_CountDirContent',

  // Scorciatoie nuove (non presenti nella mappa di fabbrica di TC)
  Apps: 'cm_ContextMenu', // tasto Menu: in TC lo gestisce Windows, non la mappa
  'Ctrl+Shift+E': 'mc_ToggleSidebar',
  'Ctrl+Oem3': 'mc_ToggleBottomPanel', // Ctrl+ò sul layout italiano
  'Ctrl+Comma': 'cm_Config',
  'Ctrl+Shift+P': 'mc_CommandPalette', // come VS Code; libero in TC
  'Ctrl+Shift+Oem5': 'mc_SplitGroup', // Ctrl+Shift+\ (come "vai alla parentesi" di VS Code; libero in TC)
};
