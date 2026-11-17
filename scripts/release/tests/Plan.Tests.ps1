# plan (changes since the last release tag) and pending-tags (tags to create).

function New-TaggedTestRepo {
    $repo = New-TestRepo
    Invoke-TestGit $repo tag -a "server-v$(Get-ComponentVersion -Root $repo -Component server)" -m server | Out-Null
    Invoke-TestGit $repo tag -a "overlay-v$(Get-ComponentVersion -Root $repo -Component overlay)" -m overlay | Out-Null
    return $repo
}

Test-Case 'Plan lists commits since tag' {
    $repo = New-TaggedTestRepo
    Add-TestCommit $repo 'apps/lc-overlay/web/x.js' 'Change the overlay'
    $overlay = Get-ComponentChanges -Root $repo -Component overlay
    Assert-Equal 1 @($overlay.Commits).Count 'overlay commits'
    Assert-Equal 'Change the overlay' @($overlay.Commits)[0].Subject 'subject'
    Assert-True (@($overlay.Files) -contains 'apps/lc-overlay/web/x.js') "files: $(@($overlay.Files) -join ', ')"
    Assert-True ($overlay.LastTag -like 'overlay-v*') "last tag: $($overlay.LastTag)"
    Assert-Equal 0 @((Get-ComponentChanges -Root $repo -Component server).Commits).Count 'server commits'
}

Test-Case 'Plan without tag lists whole history' {
    $repo = New-TaggedTestRepo
    Add-TestCommit $repo 'extension/src/a.ts' 'Change the extension'
    $ext = Get-ComponentChanges -Root $repo -Component extension
    Assert-Equal $null $ext.LastTag 'last tag'
    Assert-Equal 2 @($ext.Commits).Count 'extension commits'
    Assert-True (@($ext.Files) -contains 'extension/src/a.ts') "files: $(@($ext.Files) -join ', ')"
}

Test-Case 'Shared crates flag' {
    $repo = New-TaggedTestRepo
    Assert-Equal $false (Get-ComponentChanges -Root $repo -Component overlay).SharedCratesChanged 'before'
    Add-TestCommit $repo 'crates/lc-protocol/src/lib.rs' 'Change the protocol'
    Assert-Equal $true (Get-ComponentChanges -Root $repo -Component overlay).SharedCratesChanged 'after'
    Assert-Equal $false (Get-ComponentChanges -Root $repo -Component server).SharedCratesChanged 'server never flags'
}

Test-Case 'Pending tags after bump' {
    $repo = New-TaggedTestRepo
    $base = (Invoke-TestGit $repo rev-parse HEAD | Select-Object -First 1).Trim()
    Set-ComponentVersion -Root $repo -Component overlay -Version '90.0.0' | Out-Null
    Invoke-TestGit $repo commit -q -am 'Release overlay 90.0.0' | Out-Null
    $pending = @(Get-PendingReleaseTags -Root $repo -BaseRef $base)
    Assert-Equal 1 $pending.Count 'pending count'
    Assert-Equal 'overlay' $pending[0].Component 'component'
    Assert-Equal '90.0.0' $pending[0].Version 'version'
    Assert-Equal 'overlay-v90.0.0' $pending[0].Tag 'tag'
    Assert-Equal $false $pending[0].TagExists 'tag exists'
}

Test-Case 'Pending tags skips a tag on another commit' {
    $repo = New-TaggedTestRepo
    $base = (Invoke-TestGit $repo rev-parse HEAD | Select-Object -First 1).Trim()
    Set-ComponentVersion -Root $repo -Component overlay -Version '90.0.0' | Out-Null
    Invoke-TestGit $repo commit -q -am 'Release overlay 90.0.0' | Out-Null
    Invoke-TestGit $repo tag -a 'overlay-v90.0.0' -m overlay $base | Out-Null
    Assert-Equal 0 @(Get-PendingReleaseTags -Root $repo -BaseRef $base).Count 'pending count'
}

Test-Case 'Pending tags keeps a tag on HEAD for a retry' {
    # A re-run after a failed build dispatch must dispatch again.
    $repo = New-TaggedTestRepo
    $base = (Invoke-TestGit $repo rev-parse HEAD | Select-Object -First 1).Trim()
    Set-ComponentVersion -Root $repo -Component overlay -Version '90.0.0' | Out-Null
    Invoke-TestGit $repo commit -q -am 'Release overlay 90.0.0' | Out-Null
    Invoke-TestGit $repo tag -a 'overlay-v90.0.0' -m overlay | Out-Null
    $pending = @(Get-PendingReleaseTags -Root $repo -BaseRef $base)
    Assert-Equal 1 $pending.Count 'pending count'
    Assert-Equal $true $pending[0].TagExists 'tag exists'
}

Test-Case 'No bump no tag' {
    $repo = New-TaggedTestRepo
    $base = (Invoke-TestGit $repo rev-parse HEAD | Select-Object -First 1).Trim()
    Add-TestCommit $repo 'extension/src/a.ts' 'Change the extension'
    Assert-Equal 0 @(Get-PendingReleaseTags -Root $repo -BaseRef $base).Count 'pending count'
}
