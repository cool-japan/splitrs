;;; splitrs.el --- LSP client for splitrs-lsp (Rust refactoring assistant) -*- lexical-binding: t -*-

;; Copyright (C) 2024-2026 COOLJAPAN OU
;; Author: COOLJAPAN OU (Team KitaSan) <kitahata@terra-intl.com>
;; URL: https://github.com/cool-japan/splitrs
;; Version: 0.1.0
;; Package-Requires: ((emacs "29.1"))
;; Keywords: languages, tools, rust, lsp, refactoring

;;; Commentary:

;; splitrs.el — Emacs integration for splitrs-lsp
;;
;; Provides eglot and lsp-mode support for splitrs-lsp, the Language Server
;; that ships with the `splitrs' Rust refactoring tool.
;;
;; splitrs-lsp offers:
;;   * INFORMATION-severity diagnostics when a Rust file exceeds the
;;     configured `max_lines' limit (source: "splitrs", code: "oversize")
;;   * A "Refactor with splitrs" code action (REFACTOR_REWRITE kind) that
;;     splits the file in-place using a WorkspaceEdit
;;   * Hover at the top of the file showing metrics (LoC, method count,
;;     avg complexity)
;;
;; INSTALLATION
;; ============
;;
;; Prerequisite: install splitrs-lsp via `cargo install splitrs'
;;
;; Option A — eglot (built-in, Emacs 29.1+):
;;
;;   (require 'splitrs)
;;   (splitrs-setup)   ; or (add-hook 'rust-mode-hook #'splitrs-setup) -- not
;;                     ; needed if using splitrs-mode minor mode
;;
;; Option B — lsp-mode (external):
;;
;;   (require 'splitrs)
;;   (splitrs-setup)   ; registers the lsp-mode client
;;
;; Option C — use-package:
;;
;;   (use-package splitrs
;;     :load-path "path/to/splitrs/editors/emacs"
;;     :hook ((rust-mode . splitrs-mode)
;;            (rust-ts-mode . splitrs-mode)))
;;
;; USAGE
;; =====
;;
;; Once configured, open any Rust file.  splitrs-lsp activates automatically.
;; Use M-x splitrs-refactor-current-file (or bind it) to trigger the split.
;;
;; COEXISTENCE WITH RUST-ANALYZER
;; ================================
;;
;; eglot: both servers can run per-buffer using eglot's multi-server support.
;;   See https://github.com/joaotavora/eglot#multiple-servers-per-buffer
;;   When using eglot, M-x eglot will start the *first* matching server; to
;;   start splitrs-lsp specifically, use M-x eglot with a prefix argument or
;;   configure eglot-server-programs with a list.
;;
;; lsp-mode: splitrs is registered with :add-on? t, which allows it to run
;;   alongside lsp-rust-analyzer without displacing it.

;;; Code:

(require 'cl-lib)
(require 'map)

;; ---------------------------------------------------------------------------
;; Customization
;; ---------------------------------------------------------------------------

(defgroup splitrs nil
  "Emacs integration for splitrs-lsp."
  :group 'lsp
  :group 'tools
  :prefix "splitrs-"
  :link '(url-link "https://github.com/cool-japan/splitrs"))

(defcustom splitrs-server-path "splitrs-lsp"
  "Path to the splitrs-lsp binary.
Defaults to `splitrs-lsp' on $PATH (installed via `cargo install splitrs')."
  :type 'string
  :group 'splitrs)

(defcustom splitrs-enable-eglot t
  "When non-nil, register splitrs-lsp with eglot if eglot is available."
  :type 'boolean
  :group 'splitrs)

(defcustom splitrs-enable-lsp-mode t
  "When non-nil, register splitrs-lsp with lsp-mode if lsp-mode is available."
  :type 'boolean
  :group 'splitrs)

(defcustom splitrs-major-modes '(rust-mode rust-ts-mode)
  "Major modes for which splitrs-lsp should be activated."
  :type '(repeat symbol)
  :group 'splitrs)

;; ---------------------------------------------------------------------------
;; eglot integration
;; ---------------------------------------------------------------------------

(defun splitrs--register-eglot ()
  "Register splitrs-lsp with eglot for Rust major modes.

Uses `add-to-list' with APPEND=t so the entry goes at the end of
`eglot-server-programs'.  eglot selects the first matching entry, so
rust-analyzer (if configured) remains the primary server; users who want
splitrs-lsp as the primary server should place it first manually.

Coexistence via multiple servers per buffer is documented at:
  https://github.com/joaotavora/eglot#multiple-servers-per-buffer"
  (when (and splitrs-enable-eglot (boundp 'eglot-server-programs))
    (add-to-list
     'eglot-server-programs
     (cons splitrs-major-modes (list splitrs-server-path))
     t  ; append — don't displace rust-analyzer at the front
     )))

;; Register when eglot loads (or immediately if already loaded)
(with-eval-after-load 'eglot
  (splitrs--register-eglot))

;; ---------------------------------------------------------------------------
;; lsp-mode integration
;; ---------------------------------------------------------------------------

(defun splitrs--register-lsp-mode ()
  "Register splitrs-lsp with lsp-mode.

Uses :add-on? t so splitrs-lsp runs alongside lsp-rust-analyzer without
displacing it.  This is the critical flag for coexistence."
  (when (and splitrs-enable-lsp-mode
             (fboundp 'lsp-register-client)
             (fboundp 'make-lsp-client))
    (lsp-register-client
     (make-lsp-client
      :new-connection (lsp-stdio-connection (lambda () splitrs-server-path))
      :activation-fn (lsp-activate-on "rust")
      :server-id 'splitrs-lsp
      :priority -2   ; run after rust-analyzer (priority -1) by default
      :add-on? t     ; coexist — do NOT replace lsp-rust-analyzer
      :notification-handlers
      (ht ("window/showMessage"
           (lambda (_workspace params)
             (let ((type (gethash "type" params))
                   (msg  (gethash "message" params "")))
               (cond
                ((= type 1) (lsp--error "%s" msg))
                ((= type 2) (lsp--warn  "%s" msg))
                (t          (lsp--info  "%s" msg))))))
          ("window/logMessage"
           (lambda (_workspace params)
             (lsp--log-io-event 'notification "splitrs" params))))
      :download-server-fn nil))))

;; Register when lsp-mode loads (or immediately if already loaded)
(with-eval-after-load 'lsp-mode
  (splitrs--register-lsp-mode))

;; ---------------------------------------------------------------------------
;; Interactive command
;; ---------------------------------------------------------------------------

;;;###autoload
(defun splitrs-refactor-current-file ()
  "Invoke splitrs.split on the current buffer via workspace/executeCommand.

The server applies a WorkspaceEdit directly (via client.apply_edit), so no
client-side edit application is needed.  Works with both eglot and lsp-mode."
  (interactive)
  (let ((file (buffer-file-name)))
    (unless file
      (user-error "splitrs: current buffer has no associated file"))
    (let ((uri (concat "file://" (expand-file-name file))))
      (cond
       ;; eglot path
       ((and (featurep 'eglot) (fboundp 'eglot-current-server) (eglot-current-server))
        (eglot-execute-command
         (eglot-current-server)
         "splitrs.split"
         (vector (list :uri uri))))
       ;; lsp-mode path
       ((and (featurep 'lsp-mode) (fboundp 'lsp-send-execute-command))
        (lsp-send-execute-command "splitrs.split" (list :uri uri)))
       (t
        (user-error "splitrs: no active LSP client found (start eglot or lsp-mode first)"))))))

;; ---------------------------------------------------------------------------
;; Minor mode (optional convenience)
;; ---------------------------------------------------------------------------

;;;###autoload
(define-minor-mode splitrs-mode
  "Minor mode that ensures splitrs-lsp is attached to the current buffer.

Enabling `splitrs-mode' in a Rust buffer starts splitrs-lsp (via eglot or
lsp-mode, whichever is active) if it is not already running."
  :lighter " SplitRS"
  :group 'splitrs
  (when splitrs-mode
    (cond
     ((and (featurep 'eglot) (fboundp 'eglot-ensure))
      (eglot-ensure))
     ((and (featurep 'lsp-mode) (fboundp 'lsp-deferred))
      (lsp-deferred)))))

;; ---------------------------------------------------------------------------
;; Public setup entry point
;; ---------------------------------------------------------------------------

;;;###autoload
(defun splitrs-setup ()
  "Register splitrs-lsp with eglot and/or lsp-mode.

Call this after requiring the package if you prefer not to use the minor mode."
  (splitrs--register-eglot)
  (splitrs--register-lsp-mode))

(provide 'splitrs)
;;; splitrs.el ends here
