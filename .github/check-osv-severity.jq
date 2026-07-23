def high_or_critical:
  ((.database_specific.severity? // "" | ascii_upcase) as $severity
    | $severity == "HIGH" or $severity == "CRITICAL")
  or any(.affected[]?.database_specific.cvss?; type == "number" and . >= 7);

[
  .results[].packages[].vulnerabilities[]?
  | select(high_or_critical)
  | .id
]
| unique
| if length == 0 then
    "No high or critical advisories found"
  else
    error("High or critical advisories found: \(join(", "))")
  end
