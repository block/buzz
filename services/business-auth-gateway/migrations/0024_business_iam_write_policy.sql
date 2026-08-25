-- Register future write capabilities and their non-bypassable default controls.
-- No runtime route accepts these capabilities yet: execution remains V7_BLOCKED.
INSERT INTO business_iam.permissions(
  id,capability,resource_type,action,obligations,risk_level
)
VALUES
  (gen_random_uuid(),'sales_order:write','sales_order','write',
   '["human_approval","step_up_authentication","dual_control"]'::jsonb,'high'),
  (gen_random_uuid(),'purchase_order:write','purchase_order','write',
   '["human_approval","step_up_authentication","dual_control"]'::jsonb,'high'),
  (gen_random_uuid(),'inventory:adjust','inventory','adjust',
   '["human_approval","step_up_authentication","dual_control"]'::jsonb,'critical'),
  (gen_random_uuid(),'payment:execute','payment','execute',
   '["human_approval","step_up_authentication","dual_control"]'::jsonb,'critical'),
  (gen_random_uuid(),'business_approval:approve','business_approval','approve',
   '["step_up_authentication","dual_control"]'::jsonb,'high');
