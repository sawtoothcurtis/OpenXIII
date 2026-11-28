package org.openxiii;

import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import android.text.Editable;
import android.text.InputType;
import android.text.Selection;
import android.text.SpannableStringBuilder;
import android.view.KeyEvent;
import android.view.View;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.ExtractedText;
import android.view.inputmethod.ExtractedTextRequest;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;

/**
 * The phone's keyboard for the menus' text fields (profile name, multiplayer names and
 * passwords): open while such a field has the focus in the game (NativeInput.textField()), as a
 * PC keyboard can type in it. The text typed is sent to the game as the PC's typed characters
 * (WM_CHAR, XIIIEditCtrl.InternalOnKeyType) and Backspace and Enter keys
 * (XIIIEditCtrl.InternalOnKeyEvent): the game keeps its own rules (allowed characters, length).
 * Only the menus: nothing while playing.
 */
final class TextKeyboard extends View {
    private final InputMethodManager imm;
    private final Handler handler = new Handler(Looper.getMainLooper());
    /** Text shown by the keyboard (its own text box in landscape). */
    private final SpannableStringBuilder editable = new SpannableStringBuilder();
    /** Text the game has, as far as we sent it. */
    private String sent = "";
    /** NativeInput.textField() of the field the keyboard is for (0: none). */
    private int field;
    /** Keyboard closed by the player (Back) or by Enter: opened again by a tap. */
    private boolean dismissed;

    TextKeyboard(Context context) {
        super(context);
        imm = (InputMethodManager) context.getSystemService(Context.INPUT_METHOD_SERVICE);
        setFocusable(true);
        setFocusableInTouchMode(true);
    }

    /** Called regularly (UI thread): opens or closes the keyboard as the game's focus changes. */
    void poll() {
        int f;
        try {
            f = NativeInput.textField();
        } catch (UnsatisfiedLinkError e) {
            f = 0;
        }
        if (f == field) return;
        field = f;
        dismissed = false;
        if (f == 0) {
            hide();
        } else {
            show();
        }
    }

    /** A finger lifted on the screen: on the text field again, the keyboard comes back. */
    void onTap() {
        if (field == 0) return;
        final int f = field;
        handler.postDelayed(() -> {
            // Closed by Back, Enter or the keyboard's own button (which tells the app nothing).
            if (field == f && (dismissed || !imeVisible()) && NativeInput.textField() == f) {
                dismissed = false;
                show();
            }
        }, 250);
    }

    /** Whether the keyboard is on screen (before Android 11: unknown, taken as closed). */
    private boolean imeVisible() {
        if (android.os.Build.VERSION.SDK_INT < 30) return false;
        android.view.WindowInsets insets = getRootWindowInsets();
        return insets != null && insets.isVisible(android.view.WindowInsets.Type.ime());
    }

    private void show() {
        reset(NativeInput.textFieldText());
        requestFocus();
        imm.restartInput(this);
        imm.showSoftInput(this, 0);
    }

    private void hide() {
        handler.removeCallbacks(check);
        imm.hideSoftInputFromWindow(getWindowToken(), 0);
        clearFocus();
    }

    private void reset(String text) {
        editable.clear();
        editable.append(text);
        Selection.setSelection(editable, editable.length());
        sent = text;
    }

    /** Sends the difference between the keyboard's text and the game's: Backspaces, then characters. */
    private void sync() {
        if (field == 0) return;
        String now = editable.toString();
        int p = 0;
        while (p < now.length() && p < sent.length() && now.charAt(p) == sent.charAt(p)) p++;
        for (int i = sent.length(); i > p; i--) {
            NativeInput.key(0x08, true);
            NativeInput.key(0x08, false);
        }
        for (int i = p; i < now.length(); i++) NativeInput.character(now.charAt(i));
        sent = now;
        handler.removeCallbacks(check);
        handler.postDelayed(check, 200);
        // As an EditText does: the keyboard learns the new text and cursor once its edit is over.
        if (batch > 0) {
            reportPending = true;
        } else {
            reportExtracted();
        }
    }

    /** Keyboard's batch edits in progress (beginBatchEdit / endBatchEdit). */
    private int batch;
    private boolean reportPending;

    /** The game refused characters (not allowed, too long): the keyboard shows the game's text. */
    private final Runnable check = new Runnable() {
        @Override
        public void run() {
            if (field == 0 || NativeInput.textField() != field) return;
            String game = NativeInput.textFieldText();
            if (!game.equals(sent)) {
                reset(game);
                if (monitoring) imm.updateExtractedText(TextKeyboard.this, extractToken, extracted());
                imm.restartInput(TextKeyboard.this);
            }
        }
    };

    /** The keyboard's own text box (landscape, extract mode) follows this one when it asked to. */
    private boolean monitoring;
    private int extractToken;

    /** The text for the keyboard's own text box: the field's text and its cursor. */
    private ExtractedText extracted() {
        ExtractedText t = new ExtractedText();
        t.text = editable;
        t.startOffset = 0;
        t.partialStartOffset = -1;
        t.partialEndOffset = -1;
        t.selectionStart = Selection.getSelectionStart(editable);
        t.selectionEnd = Selection.getSelectionEnd(editable);
        return t;
    }

    /** Text changed here, not by the keyboard (keys, game's text): its text box shows it. */
    private void reportExtracted() {
        if (monitoring) imm.updateExtractedText(this, extractToken, extracted());
        imm.updateSelection(this, Selection.getSelectionStart(editable), Selection.getSelectionEnd(editable),
                BaseInputConnection.getComposingSpanStart(editable), BaseInputConnection.getComposingSpanEnd(editable));
    }

    private void enter() {
        NativeInput.key(0x0D, true);
        NativeInput.key(0x0D, false);
        dismissed = true;
        hide();
    }

    @Override
    public boolean onCheckIsTextEditor() {
        return field != 0;
    }

    @Override
    public boolean onKeyPreIme(int keyCode, KeyEvent event) {
        // Back while the keyboard is open: Android closes it; the game stays in its menu.
        if (keyCode == KeyEvent.KEYCODE_BACK && event.getAction() == KeyEvent.ACTION_UP) dismissed = true;
        return super.onKeyPreIme(keyCode, event);
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo out) {
        if (field == 0) return null;
        monitoring = false;
        batch = 0;
        reportPending = false;
        out.inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                | ((field & 1) != 0 ? InputType.TYPE_TEXT_VARIATION_PASSWORD : 0);
        out.imeOptions = EditorInfo.IME_ACTION_DONE;
        out.initialSelStart = editable.length();
        out.initialSelEnd = editable.length();
        return new Connection();
    }

    private final class Connection extends BaseInputConnection {
        Connection() {
            super(TextKeyboard.this, true);
        }

        @Override
        public Editable getEditable() {
            return editable;
        }

        /** Landscape: the keyboard's text box shows the field's text (empty without this). */
        @Override
        public ExtractedText getExtractedText(ExtractedTextRequest request, int flags) {
            if ((flags & InputConnection.GET_EXTRACTED_TEXT_MONITOR) != 0) {
                monitoring = true;
                extractToken = request != null ? request.token : 0;
            }
            return extracted();
        }

        @Override
        public boolean beginBatchEdit() {
            batch++;
            return super.beginBatchEdit();
        }

        @Override
        public boolean endBatchEdit() {
            boolean r = super.endBatchEdit();
            if (batch > 0) batch--;
            sync();
            if (batch == 0 && reportPending) {
                reportPending = false;
                reportExtracted();
            }
            return r;
        }

        @Override
        public boolean commitText(CharSequence text, int newCursorPosition) {
            boolean r = super.commitText(text, newCursorPosition);
            sync();
            return r;
        }

        @Override
        public boolean setComposingText(CharSequence text, int newCursorPosition) {
            boolean r = super.setComposingText(text, newCursorPosition);
            sync();
            return r;
        }

        @Override
        public boolean deleteSurroundingText(int before, int after) {
            boolean r = super.deleteSurroundingText(before, after);
            sync();
            return r;
        }

        @Override
        public boolean performEditorAction(int action) {
            enter();
            return true;
        }

        /** Keys sent by some keyboards instead of text. */
        @Override
        public boolean sendKeyEvent(KeyEvent e) {
            if (e.getAction() != KeyEvent.ACTION_DOWN) return true;
            int code = e.getKeyCode();
            if (code == KeyEvent.KEYCODE_ENTER || code == KeyEvent.KEYCODE_NUMPAD_ENTER) {
                enter();
            } else if (code == KeyEvent.KEYCODE_DEL) {
                int end = Selection.getSelectionEnd(editable);
                if (end > 0) {
                    editable.delete(end - 1, end);
                    sync();
                }
            } else {
                int u = e.getUnicodeChar();
                if (u > 0) {
                    int end = Math.max(0, Selection.getSelectionEnd(editable));
                    editable.insert(end, String.valueOf((char) u));
                    sync();
                }
            }
            return true;
        }
    }
}
