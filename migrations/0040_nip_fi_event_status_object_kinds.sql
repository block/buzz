-- Add the two application-owned canonical mutation namespaces.

ALTER TABLE authorization_admission_results
    DROP CONSTRAINT authorization_admission_results_object_kind_check;
ALTER TABLE authorization_admission_results
    ADD CONSTRAINT authorization_admission_results_object_kind_check
    CHECK (object_kind IN (1, 2, 3, 4, 5, 6, 7, 8, 9)) NOT VALID;
ALTER TABLE authorization_admission_results
    VALIDATE CONSTRAINT authorization_admission_results_object_kind_check;

ALTER TABLE authorization_authority_epochs
    DROP CONSTRAINT authorization_authority_epochs_object_kind_check;
ALTER TABLE authorization_authority_epochs
    ADD CONSTRAINT authorization_authority_epochs_object_kind_check
    CHECK (object_kind IN (1, 2, 3, 4, 5, 6, 7, 8, 9)) NOT VALID;
ALTER TABLE authorization_authority_epochs
    VALIDATE CONSTRAINT authorization_authority_epochs_object_kind_check;

ALTER TABLE protected_object_authority
    DROP CONSTRAINT protected_object_authority_object_kind_check;
ALTER TABLE protected_object_authority
    ADD CONSTRAINT protected_object_authority_object_kind_check
    CHECK (object_kind IN (1, 2, 3, 4, 5, 6, 7, 8, 9)) NOT VALID;
ALTER TABLE protected_object_authority
    VALIDATE CONSTRAINT protected_object_authority_object_kind_check;
