;;; splitrs-test.el --- ERT tests for splitrs.el -*- lexical-binding: t -*-

;;; Commentary:
;; Run with:
;;   emacs --batch -Q -L editors/emacs \
;;         -l ert -l editors/emacs/splitrs.el \
;;         -l editors/emacs/test/splitrs-test.el \
;;         -f ert-run-tests-batch-and-exit

;;; Code:

(require 'ert)
(require 'splitrs)

(ert-deftest splitrs-test-loads ()
  "Package should be providable."
  (should (featurep 'splitrs)))

(ert-deftest splitrs-test-defcustoms-exist ()
  "All expected defcustoms should be bound with correct types."
  (should (boundp 'splitrs-server-path))
  (should (stringp splitrs-server-path))
  (should (string= splitrs-server-path "splitrs-lsp"))
  (should (boundp 'splitrs-enable-eglot))
  (should (eq splitrs-enable-eglot t))
  (should (boundp 'splitrs-enable-lsp-mode))
  (should (eq splitrs-enable-lsp-mode t))
  (should (boundp 'splitrs-major-modes))
  (should (listp splitrs-major-modes))
  (should (memq 'rust-mode splitrs-major-modes))
  (should (memq 'rust-ts-mode splitrs-major-modes)))

(ert-deftest splitrs-test-refactor-fn-defined ()
  "splitrs-refactor-current-file must be an interactive function."
  (should (fboundp 'splitrs-refactor-current-file))
  (should (commandp 'splitrs-refactor-current-file)))

(ert-deftest splitrs-test-setup-fn-defined ()
  "splitrs-setup must be callable."
  (should (fboundp 'splitrs-setup))
  (should (commandp 'splitrs-setup)))

(ert-deftest splitrs-test-minor-mode-defined ()
  "splitrs-mode minor mode must exist."
  (should (fboundp 'splitrs-mode))
  (should (boundp 'splitrs-mode)))

(ert-deftest splitrs-test-eglot-registration ()
  "After loading eglot, splitrs should appear in eglot-server-programs."
  (skip-unless (require 'eglot nil t))
  ;; Trigger registration
  (splitrs--register-eglot)
  (should (boundp 'eglot-server-programs))
  (let ((entry (cl-find-if
                (lambda (e)
                  (and (consp e)
                       (or (and (listp (car e)) (memq 'rust-mode (car e)))
                           (equal (car e) 'rust-mode))))
                eglot-server-programs)))
    (should entry)
    (should (member splitrs-server-path (if (listp (cdr entry))
                                            (cdr entry)
                                          (list (cdr entry)))))))

(ert-deftest splitrs-test-register-eglot-fn-defined ()
  "Internal registration function must exist."
  (should (fboundp 'splitrs--register-eglot)))

(ert-deftest splitrs-test-register-lsp-mode-fn-defined ()
  "Internal lsp-mode registration function must exist."
  (should (fboundp 'splitrs--register-lsp-mode)))

;;; splitrs-test.el ends here
