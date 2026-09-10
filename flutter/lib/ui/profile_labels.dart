const profileSettingLabels = {
  'appearance': 'Appearance',
  'left_swipe': 'Swipe left',
  'right_swipe': 'Swipe right',
  'preview_lines': 'Preview lines',
  'sender_pictures': 'Sender pictures',
  'unified_inbox': 'Unified inbox',
  'reply_display': 'Quoted history',
  'tooltips': 'Tooltips',
};
String profileSettingLabel(String field) =>
    profileSettingLabels[field] ?? field;
String profileValueText(Object? value) => value == null
    ? 'Reset to default'
    : value is bool
    ? (value ? 'On' : 'Off')
    : '$value';
