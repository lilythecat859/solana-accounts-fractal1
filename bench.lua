-- bench.lua – wrk script for getProgramAccounts
wrk.method = "POST"
wrk.body   = '{"program":"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"}'
wrk.headers["Content-Type"] = "application/json"
