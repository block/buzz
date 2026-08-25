-- Register human-managed execution permissions and their non-bypassable controls.
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
